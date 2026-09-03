// Model-free smoke executable for the actual Rust -> CXX backend initialization.
#[allow(dead_code)]
#[path = "../src-tauri/src/native_bridge.rs"]
mod native_bridge;

fn main() {
    let mode = std::env::args().nth(1).expect("cpu or gpu-unavailable");
    let layers = match mode.as_str() {
        "cpu" => 0,
        "gpu-unavailable" => 1,
        _ => panic!("unknown backend probe mode"),
    };
    let missing = std::env::temp_dir().join(format!(
        "openmind-missing-model-{}.gguf",
        std::process::id()
    ));
    assert!(!missing.exists(), "probe model path must not exist");
    let error = match native_bridge::NativeInferenceEngine::load(&missing.to_string_lossy(), layers) {
        Ok(_) => panic!("nonexistent model unexpectedly loaded"),
        Err(error) => error.to_string(),
    };
    let expected = if layers == 0 {
        "failed to load GGUF model"
    } else {
        "native Vulkan backend is unavailable; CPU retry required"
    };
    assert_eq!(error, expected, "unexpected backend initialization failure");
    println!("native.wrapper: {mode} passed");
}
