use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=LLAMA_CPP_DIR");
    println!("cargo:rerun-if-env-changed=LLAMA_CPP_LIB_DIR");
    println!("cargo:rerun-if-env-changed=OPENMINDAI_LLAMA_LINK_KIND");
    println!("cargo:rerun-if-env-changed=OPENMINDAI_LLAMA_EXTRA_LIBS");
    println!("cargo:rerun-if-env-changed=OPENMINDAI_LLAMA_CUDA");
    println!("cargo:rerun-if-env-changed=OPENMINDAI_PORTABLE_BUILD");
    println!("cargo:rerun-if-env-changed=CUDA_PATH");
    println!("cargo:rerun-if-env-changed=CUDA_HOME");
    println!("cargo:rerun-if-changed=native/inference.cpp");
    println!("cargo:rerun-if-changed=native/inference.h");
    println!("cargo:rerun-if-changed=src/native_bridge.rs");

    if env::var_os("CARGO_FEATURE_NATIVE_CXX_LLAMA").is_some() {
        build_native_llama_bridge();
    }

    tauri_build::build();
}

fn build_native_llama_bridge() {
    let llama_dir = required_path("LLAMA_CPP_DIR");
    let llama_lib_dir = required_path("LLAMA_CPP_LIB_DIR");
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let portable = env_flag("OPENMINDAI_PORTABLE_BUILD");

    cxx_build::CFG.include_prefix = "openmind";

    let mut build = cxx_build::bridge("src/native_bridge.rs");
    build
        .file("native/inference.cpp")
        .include(".")
        .include(llama_dir.join("include"))
        .include(llama_dir.join("ggml/include"))
        .std("c++17")
        .warnings(true);

    if target_env == "msvc" {
        build.flag("/O2").flag("/EHsc");
        if !portable && (target_arch == "x86" || target_arch == "x86_64") {
            build.flag_if_supported("/arch:AVX2");
        }
    } else {
        build.flag("-O3").flag_if_supported("-fPIC");
        if !portable {
            if target_arch == "aarch64" {
                build.flag_if_supported("-mcpu=native");
            } else {
                build.flag_if_supported("-march=native");
            }
        }
    }

    // This compiles only OpenMindAI's thin wrapper. CUDA/AVX kernels are compiled
    // inside llama.cpp itself, so build llama.cpp with GGML_CUDA=ON / native CPU
    // flags and point this crate at that output directory.
    build.compile("openmind_llama_bridge");

    println!("cargo:rustc-link-search=native={}", llama_lib_dir.display());
    let link_kind = env::var("OPENMINDAI_LLAMA_LINK_KIND").unwrap_or_else(|_| "dylib".to_string());
    match link_kind.as_str() {
        "static" | "dylib" => println!("cargo:rustc-link-lib={link_kind}=llama"),
        other => panic!("OPENMINDAI_LLAMA_LINK_KIND must be 'static' or 'dylib', got {other}"),
    }

    if let Ok(extra) = env::var("OPENMINDAI_LLAMA_EXTRA_LIBS") {
        for lib in extra
            .split([';', ','])
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            println!("cargo:rustc-link-lib={lib}");
        }
    }

    if env_flag("OPENMINDAI_LLAMA_CUDA") {
        configure_cuda_link_search(&target_env);
        println!("cargo:rustc-cfg=openmind_llama_cuda");
    }
}

fn required_path(name: &str) -> PathBuf {
    let value = env::var_os(name).unwrap_or_else(|| {
        panic!("{name} is required when building with --features native-cxx-llama")
    });
    let path = PathBuf::from(value);
    if !path.exists() {
        panic!("{name} does not exist: {}", path.display());
    }
    path
}

fn env_flag(name: &str) -> bool {
    matches!(
        env::var(name).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
    )
}

fn configure_cuda_link_search(target_env: &str) {
    let Some(cuda_root) = env::var_os("CUDA_PATH").or_else(|| env::var_os("CUDA_HOME")) else {
        println!("cargo:warning=OPENMINDAI_LLAMA_CUDA=1 but CUDA_PATH/CUDA_HOME is not set; relying on the system linker path");
        return;
    };

    let root = PathBuf::from(cuda_root);
    let candidates = if target_env == "msvc" {
        vec![root.join("lib/x64")]
    } else {
        vec![root.join("lib64"), root.join("lib")]
    };
    for path in candidates.into_iter().filter(|path| path.is_dir()) {
        println!("cargo:rustc-link-search=native={}", path.display());
    }
}
