use std::{env, path::PathBuf};

fn main() {
    let root = PathBuf::from(env::var_os("LLAMA_CPP_DIR").unwrap());
    cxx_build::CFG.include_prefix = "openmind";
    cxx_build::bridge("src/native_bridge.rs")
        .file("native/inference.cpp")
        .include(".")
        .include(root.join("include"))
        .include(root.join("ggml/include"))
        .std("c++17")
        .flag("/O2")
        .flag("/EHsc")
        .define("OPENMINDAI_DYNAMIC_BACKENDS", None)
        .compile("openmind_llama_bridge");
    for name in ["LLAMA_CPP_LIB_DIR", "LLAMA_CPP_BACKEND_LIB_DIR"] {
        println!("cargo:rustc-link-search=native={}", env::var(name).unwrap());
    }
    println!("cargo:rustc-link-lib=dylib=llama");
    println!("cargo:rustc-link-lib=dylib=ggml");
    println!("cargo:rustc-link-lib=dylib=ggml-base");
}
