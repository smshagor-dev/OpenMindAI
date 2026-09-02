use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    println!("cargo:rerun-if-env-changed=OPENMINDAI_LLAMA_DIR");
    println!("cargo:rerun-if-env-changed=OPENMINDAI_LLAMA_CUDA");
    println!("cargo:rerun-if-env-changed=OPENMINDAI_NATIVE_TUNE");
    println!("cargo:rerun-if-changed=src/bridge.rs");
    println!("cargo:rerun-if-changed=cpp/inference.cpp");
    println!("cargo:rerun-if-changed=include/inference.h");

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let llama_dir = env::var_os("OPENMINDAI_LLAMA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.join("vendor").join("llama.cpp"));

    if !llama_dir.join("CMakeLists.txt").is_file() {
        panic!(
            "llama.cpp source not found at {}. Set OPENMINDAI_LLAMA_DIR or place llama.cpp in native-core/vendor/llama.cpp",
            llama_dir.display()
        );
    }

    let target = env::var("TARGET").unwrap_or_default();
    let is_windows = target.contains("windows");
    let is_msvc = target.contains("msvc");
    let is_x86_64 = target.contains("x86_64");
    let native_tune = env_flag("OPENMINDAI_NATIVE_TUNE").unwrap_or(false);
    let cuda = env_flag("OPENMINDAI_LLAMA_CUDA").unwrap_or_else(|| nvcc_available(is_windows));

    let mut llama = cmake::Config::new(&llama_dir);
    llama
        .profile("Release")
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("LLAMA_BUILD_COMMON", "OFF")
        .define("LLAMA_BUILD_EXAMPLES", "OFF")
        .define("LLAMA_BUILD_SERVER", "OFF")
        .define("LLAMA_BUILD_TESTS", "OFF")
        .define("GGML_NATIVE", if native_tune { "ON" } else { "OFF" });

    if is_x86_64 && !native_tune {
        // Portable production baseline. AVX-512 remains opt-in through
        // OPENMINDAI_NATIVE_TUNE=1 so release binaries do not SIGILL on
        // machines that only support AVX2.
        llama.define("GGML_AVX", "ON");
        llama.define("GGML_AVX2", "ON");
        llama.define("GGML_AVX512", "OFF");
    }

    if cuda {
        llama
            .define("GGML_CUDA", "ON")
            .define("CMAKE_CUDA_FLAGS_RELEASE", "-O3 --use_fast_math");
    } else {
        llama.define("GGML_CUDA", "OFF");
    }

    let llama_out = llama.build();
    emit_link_searches(&llama_out);

    // llama.cpp static targets. The exact file extension is selected by rustc.
    for library in ["llama", "ggml", "ggml-base", "ggml-cpu"] {
        println!("cargo:rustc-link-lib=static={library}");
    }
    if cuda {
        println!("cargo:rustc-link-lib=static=ggml-cuda");
        println!("cargo:rustc-link-lib=dylib=cudart");
        println!("cargo:rustc-link-lib=dylib=cublas");
        println!("cargo:rustc-link-lib=dylib=cublasLt");
    }

    let mut bridge = cxx_build::bridge("src/bridge.rs");
    bridge
        .file("cpp/inference.cpp")
        .include("include")
        .include(llama_dir.join("include"))
        .include(llama_dir.join("ggml").join("include"))
        .std("c++17");

    if is_msvc {
        bridge.flag_if_supported("/O2");
        bridge.flag_if_supported("/EHsc");
        if is_x86_64 && !native_tune {
            bridge.flag_if_supported("/arch:AVX2");
        }
    } else {
        bridge.flag_if_supported("-O3");
        bridge.flag_if_supported("-fvisibility=hidden");
        if is_x86_64 {
            if native_tune {
                bridge.flag_if_supported("-march=native");
            } else {
                bridge.flag_if_supported("-mavx2");
                bridge.flag_if_supported("-mfma");
            }
        }
    }

    bridge.compile("openmind-native-llama-bridge");
}

fn env_flag(name: &str) -> Option<bool> {
    env::var(name).ok().map(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn nvcc_available(is_windows: bool) -> bool {
    let executable = if is_windows { "nvcc.exe" } else { "nvcc" };
    Command::new(executable)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn emit_link_searches(prefix: &Path) {
    let candidates = [
        prefix.join("lib"),
        prefix.join("lib64"),
        prefix.join("build").join("bin"),
        prefix.join("build").join("bin").join("Release"),
        prefix.join("build").join("src"),
        prefix.join("build").join("ggml").join("src"),
        prefix.join("build").join("ggml").join("src").join("Release"),
    ];
    for path in candidates {
        if path.exists() {
            println!("cargo:rustc-link-search=native={}", path.display());
        }
    }
}
