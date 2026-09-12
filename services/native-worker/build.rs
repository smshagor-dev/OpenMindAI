use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    const PIN: &str = "7798007a29a90e3053e799394da48cf53a2f8e0f";
    let project = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../src-tauri")
        .canonicalize()
        .unwrap();
    let llama = PathBuf::from(env::var_os("LLAMA_CPP_DIR").expect("LLAMA_CPP_DIR"))
        .canonicalize()
        .unwrap();
    let lib = PathBuf::from(env::var_os("LLAMA_CPP_LIB_DIR").expect("LLAMA_CPP_LIB_DIR"))
        .canonicalize()
        .unwrap();
    let revision = Command::new("git")
        .arg("-C")
        .arg(&llama)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git");
    assert!(
        revision.status.success() && String::from_utf8_lossy(&revision.stdout).trim() == PIN,
        "worker requires pinned llama.cpp source"
    );
    for name in [
        "LLAMA_CPP_DIR",
        "LLAMA_CPP_LIB_DIR",
        "LLAMA_CPP_BACKEND_LIB_DIR",
        "OPENMINDAI_NATIVE_DYNAMIC_BACKENDS",
        "LLAMA_CPP_COMMIT",
        "OPENMINDAI_NATIVE_STRICT_ABI",
        "OPENMINDAI_PORTABLE_BUILD",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }

    // Stage original sources without maintaining a second bridge implementation.
    // Relative bridge paths keep the same generated CXX include prefix as Tauri.
    let stage = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("bridge-source");
    for path in [
        "src/native_bridge.rs",
        "native/inference.cpp",
        "native/inference.h",
    ] {
        let source = project.join(path);
        println!("cargo:rerun-if-changed={}", source.display());
        fs::create_dir_all(stage.join(path).parent().unwrap()).unwrap();
        fs::copy(source, stage.join(path)).unwrap();
    }
    fs::create_dir_all(stage.join("openmind/native")).unwrap();
    fs::copy(
        stage.join("native/inference.h"),
        stage.join("openmind/native/inference.h"),
    )
    .unwrap();

    let previous_dir = env::current_dir().expect("current directory");
    env::set_current_dir(&stage).unwrap();
    cxx_build::CFG.include_prefix = "openmind";
    let mut build = cxx_build::bridge("src/native_bridge.rs");
    build
        .file("native/inference.cpp")
        .include(&stage)
        .include(llama.join("include"))
        .include(llama.join("ggml/include"))
        .std("c++17");

    let windows = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if windows {
        build.flag("/O2").flag("/EHsc");
    } else {
        build.flag("-O3").flag_if_supported("-fPIC");
    }

    let dynamic_backends = env::var("OPENMINDAI_NATIVE_DYNAMIC_BACKENDS").as_deref() == Ok("1");
    if dynamic_backends {
        assert!(windows, "dynamic plugins currently require MSVC");
        let backend = PathBuf::from(
            env::var_os("LLAMA_CPP_BACKEND_LIB_DIR").expect("backend import directory"),
        )
        .canonicalize()
        .unwrap();
        for name in ["ggml", "ggml-base"] {
            assert!(
                backend.join(format!("{name}.lib")).is_file(),
                "missing backend import library: {name}.lib"
            );
        }

        // Keep search paths ahead of link directives. This matches the known-good
        // Vulkan CXX probe and avoids toolchain-dependent ordering of /LIBPATH and
        // import-library arguments on MSVC.
        println!("cargo:rustc-link-search=native={}", backend.display());
        println!("cargo:rustc-link-lib=dylib=ggml");
        println!("cargo:rustc-link-lib=dylib=ggml-base");
        build.define("OPENMINDAI_DYNAMIC_BACKENDS", None);
    }

    build.compile("openmind_llama_bridge");
    env::set_current_dir(previous_dir).expect("restore current directory");

    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=dylib=llama");
}
