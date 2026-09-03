//! Real GGUF smoke runner. Uses production bridge/backend/supervisor source files.
#[allow(dead_code)]
#[path = "../src-tauri/src/native_bridge.rs"]
mod native_bridge;
#[allow(dead_code)]
#[path = "../src-tauri/src/native_inference.rs"]
mod native_inference;
#[allow(dead_code)]
#[path = "../src-tauri/src/native_supervisor.rs"]
mod native_supervisor;

use std::{
    error::Error,
    fs,
    io::Write,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

use native_bridge::{ChatMessage, GenerationConfig};
use native_inference::InferenceRequest;
use native_supervisor::{
    NativeInferenceSupervisor, NativeModelSpec, NativeSupervisorError, NativeSupervisorState,
};
use serde_json::{json, Value};

type SmokeResult<T> = Result<T, Box<dyn Error>>;

struct Options {
    model: PathBuf,
    report: Option<PathBuf>,
    chat: bool,
    gpu_layers: i32,
    timeout: u64,
    expect_gpu_unavailable: bool,
}

impl Options {
    fn parse() -> SmokeResult<Self> {
        let mut options = Self {
            model: PathBuf::new(),
            report: None,
            chat: false,
            gpu_layers: 0,
            timeout: 180,
            expect_gpu_unavailable: false,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--expect-gpu-unavailable" {
                options.expect_gpu_unavailable = true;
                continue;
            }
            let value = args.next().ok_or("option requires a value")?;
            match arg.as_str() {
                "--model" => options.model = value.into(),
                "--report" => options.report = Some(value.into()),
                "--profile" => {
                    options.chat = match value.as_str() {
                        "chat" => true,
                        "raw" => false,
                        _ => return Err("profile must be raw or chat".into()),
                    }
                }
                "--gpu-layers" => options.gpu_layers = value.parse()?,
                "--timeout-seconds" => options.timeout = value.parse()?,
                _ => return Err(format!("unknown option: {arg}").into()),
            }
        }
        if !options.model.is_file() {
            return Err("--model must name an existing GGUF file".into());
        }
        if !(0..=999).contains(&options.gpu_layers) {
            return Err("GPU layers must be 0..999".into());
        }
        if !(1..=3600).contains(&options.timeout) {
            return Err("timeout must be 1..3600 seconds".into());
        }
        if options.expect_gpu_unavailable && options.gpu_layers != 0 {
            return Err(
                "GPU-unavailable scenario requires the subsequent run to use CPU (0 layers)".into(),
            );
        }
        Ok(options)
    }
}

fn request(chat: bool, prompt: &str) -> InferenceRequest {
    let messages = if chat {
        vec![
            ChatMessage::system("Reply briefly. Preserve Unicode text."),
            ChatMessage::user("Remember the word flower."),
            ChatMessage::assistant("flower"),
            ChatMessage::user(prompt),
        ]
    } else {
        vec![ChatMessage::user(prompt)]
    };
    InferenceRequest::from_messages(messages).with_config(GenerationConfig {
        temperature: 0.0,
        top_p: 1.0,
        max_tokens: 32,
        n_ctx: 512,
        n_batch: 64,
        n_threads: 2,
    })
}

#[derive(Default)]
struct StreamStats {
    chunks: usize,
    bytes: usize,
    first: Option<Duration>,
    cancelled_at: Option<Instant>,
}

fn generation(
    worker: &NativeInferenceSupervisor,
    model: &NativeModelSpec,
    input: InferenceRequest,
    name: &str,
    cancel: bool,
    checks: &mut Vec<Value>,
) -> SmokeResult<()> {
    let started = Instant::now();
    let cancellation = CancellationToken::new();
    let signal = cancellation.clone();
    let stats = Arc::new(Mutex::new(StreamStats::default()));
    let observed = Arc::clone(&stats);
    let result = worker.generate(
        model.clone(),
        input,
        cancellation,
        Box::new(move |chunk| {
            // Empty callbacks are cancellation polls, not text or first-token events.
            if chunk.is_empty() {
                return true;
            }
            let mut stats = observed.lock().unwrap();
            stats.first.get_or_insert_with(|| started.elapsed());
            stats.chunks += 1;
            stats.bytes += chunk.len();
            if cancel && stats.chunks == 1 {
                stats.cancelled_at = Some(Instant::now());
                signal.cancel();
            }
            true
        }),
    );
    let finished = Instant::now();
    let stats = stats.lock().unwrap();
    let expected_result = if cancel {
        matches!(result, Err(NativeSupervisorError::Cancelled)) && stats.chunks == 1
    } else {
        result.is_ok()
    };
    let ready = worker.state()
        == NativeSupervisorState::Ready {
            model_id: model.id.clone(),
        };
    let passed = expected_result && stats.bytes > 0 && ready;
    let error = result.as_ref().err().map(ToString::to_string);
    checks.push(
        json!({"name": name, "passed": passed, "chunks": stats.chunks,
        "output_bytes": stats.bytes, "elapsed_ms": started.elapsed().as_millis(),
        "first_chunk_ms": stats.first.map(|d| d.as_millis()),
        "cancel_return_ms": stats.cancelled_at.map(|t| finished.duration_since(t).as_millis()),
        "result": error, "worker_ready": ready}),
    );
    if !passed {
        return Err(format!("{name}: unexpected result/state or missing output: {error:?}").into());
    }
    Ok(())
}

fn suite(options: &Options, checks: &mut Vec<Value>) -> SmokeResult<()> {
    let worker = NativeInferenceSupervisor::start();
    let model = NativeModelSpec {
        id: "smoke-model".into(),
        path: options.model.clone(),
        n_gpu_layers: options.gpu_layers,
    };
    let short = "Once upon a time, a child found a flower. The child";
    if options.expect_gpu_unavailable {
        let mut gpu = model.clone();
        gpu.n_gpu_layers = 1;
        let count = Arc::new(Mutex::new(0_usize));
        let observed = Arc::clone(&count);
        let result = worker.generate(
            gpu,
            request(options.chat, short),
            CancellationToken::new(),
            Box::new(move |chunk| {
                if !chunk.is_empty() {
                    *observed.lock().unwrap() += 1;
                }
                true
            }),
        );
        let error = result.err().map(|e| e.to_string()).unwrap_or_default();
        let passed =
            error.contains("native Vulkan backend is unavailable") && *count.lock().unwrap() == 0;
        checks.push(
            json!({"name": "gpu_unavailable_before_output", "passed": passed, "error": error}),
        );
        if !passed {
            return Err("expected Vulkan availability failure before output".into());
        }
    }
    generation(
        &worker,
        &model,
        request(options.chat, short),
        "initial_generation",
        false,
        checks,
    )?;
    generation(
        &worker,
        &model,
        request(options.chat, short),
        "reuse_generation",
        false,
        checks,
    )?;
    generation(
        &worker,
        &model,
        request(options.chat, short),
        "cancel_after_first_chunk",
        true,
        checks,
    )?;
    generation(
        &worker,
        &model,
        request(options.chat, "Once upon a time, বাংলা 🌼. The child"),
        "unicode_after_cancel",
        false,
        checks,
    )?;
    let long = "Once upon a time, a child found a flower. ".repeat(96);
    generation(
        &worker,
        &model,
        request(options.chat, &long),
        "long_prompt",
        false,
        checks,
    )?;
    worker.clear_context()?;
    generation(
        &worker,
        &model,
        request(options.chat, short),
        "after_context_reset",
        false,
        checks,
    )?;

    // A malformed model forces unload/error/reload through the real worker.
    let bad_path =
        std::env::temp_dir().join(format!("openmind-invalid-{}.gguf", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&bad_path)?;
    struct RemoveFile(PathBuf);
    impl Drop for RemoveFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let _cleanup = RemoveFile(bad_path.clone());
    file.write_all(b"invalid GGUF smoke fixture")?;
    drop(file);
    let mut bad_model = model.clone();
    bad_model.path = bad_path;
    let count = Arc::new(Mutex::new(0_usize));
    let observed = Arc::clone(&count);
    let result = worker.generate(
        bad_model,
        request(options.chat, short),
        CancellationToken::new(),
        Box::new(move |chunk| {
            if !chunk.is_empty() {
                *observed.lock().unwrap() += 1;
            }
            true
        }),
    );
    let passed = matches!(
        result,
        Err(NativeSupervisorError::Inference(
            native_inference::InferenceError::ModelLoad(_)
        ))
    ) && *count.lock().unwrap() == 0;
    checks.push(json!({"name": "invalid_model_rejected", "passed": passed}));
    if !passed {
        return Err("invalid model was not rejected before output".into());
    }
    generation(
        &worker,
        &model,
        request(options.chat, short),
        "recovery_reload",
        false,
        checks,
    )?;
    drop(worker);
    checks.push(json!({"name": "clean_shutdown", "passed": true}));
    Ok(())
}

fn run() -> SmokeResult<()> {
    if std::env::args().any(|arg| arg == "--help") {
        println!("native-inference-smoke --model FILE [--profile raw|chat] [--gpu-layers 0..999] [--report FILE] [--timeout-seconds 180] [--expect-gpu-unavailable]");
        return Ok(());
    }
    let options = Options::parse()?;
    // Includes model load and worker Drop, which cannot yet be interrupted safely.
    let (_keep_alive, done) = mpsc::channel::<()>();
    let timeout = options.timeout;
    thread::spawn(move || {
        if matches!(
            done.recv_timeout(Duration::from_secs(timeout)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) {
            eprintln!(
                "native smoke exceeded {timeout}s; terminating hung load/generation/shutdown"
            );
            std::process::exit(124);
        }
    });
    let mut checks = Vec::new();
    let result = suite(&options, &mut checks);
    let report = json!({"schema_version": 1, "passed": result.is_ok(),
        "model_file": options.model.file_name().map(|name| name.to_string_lossy()),
        "model_bytes": fs::metadata(&options.model)?.len(),
        "profile": if options.chat { "chat" } else { "raw" }, "gpu_layers_requested": options.gpu_layers,
        "error": result.as_ref().err().map(ToString::to_string), "checks": checks,
        "scope": "native bridge and supervisor smoke; not UI routing or answer-quality validation"});
    let text = serde_json::to_string_pretty(&report)?;
    if let Some(path) = options.report {
        fs::write(path, &text)?;
    }
    println!("{text}");
    result
}

fn main() {
    if let Err(error) = run() {
        eprintln!("native inference smoke failed: {error}");
        std::process::exit(1);
    }
}
