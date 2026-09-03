//! Private, versioned stdin/stdout IPC. No listener and no client-selected paths.
#[allow(dead_code)]
#[path = "../../../src-tauri/src/native_bridge.rs"]
mod native_bridge;
#[allow(dead_code)]
#[path = "../../../src-tauri/src/native_inference.rs"]
mod native_inference;
#[allow(dead_code)]
#[path = "../../../src-tauri/src/native_supervisor.rs"]
mod native_supervisor;

use native_bridge::{ChatMessage, GenerationConfig};
use native_inference::InferenceRequest;
use native_supervisor::{NativeInferenceSupervisor, NativeModelSpec, NativeSupervisorError};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, BufRead, Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio_util::sync::CancellationToken;

const MAX_LINE: u64 = 1024 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Model {
    path: PathBuf,
    #[serde(default)]
    gpu_layers: i32,
    #[serde(default = "context_default")]
    context_size: u32,
}
fn context_default() -> u32 {
    8192
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    role: String,
    content: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    id: u64,
    model: String,
    messages: Vec<Message>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    max_tokens: Option<u32>,
    timeout_ms: Option<u32>,
}
fn event(value: Value) -> bool {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, &value)
        .and_then(|_| {
            out.write_all(b"\n").map_err(serde_json::Error::io)?;
            out.flush().map_err(serde_json::Error::io)
        })
        .is_ok()
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 || args[1] != "--models" {
        return Err("usage: openmind-native-worker --models /absolute/models.json".into());
    }
    let manifest = PathBuf::from(&args[2]);
    if !manifest.is_absolute() || fs::metadata(&manifest)?.len() > MAX_LINE {
        return Err("invalid model manifest path/size".into());
    }
    let models: BTreeMap<String, Model> = serde_json::from_slice(&fs::read(manifest)?)?;
    if models.is_empty() || models.len() > 32 {
        return Err("model manifest must contain 1..32 entries".into());
    }
    for (id, model) in &models {
        if id.is_empty()
            || id.len() > 128
            || !model.path.is_absolute()
            || !(0..=999).contains(&model.gpu_layers)
            || !(512..=32768).contains(&model.context_size)
        {
            return Err("invalid model registry entry".into());
        }
        let mut file = fs::File::open(&model.path)?;
        let mut magic = [0; 4];
        file.read_exact(&mut magic)?;
        if &magic != b"GGUF" || file.metadata()?.len() > 8 * 1024 * 1024 * 1024 {
            return Err("invalid or oversized GGUF".into());
        }
    }
    if !event(json!({"type":"ready", "protocol":1})) {
        return Ok(());
    }
    let worker = NativeInferenceSupervisor::start();
    let mut active: Option<String> = None;
    let mut cpu_only = false;
    let mut input = io::stdin().lock();
    loop {
        let mut line = Vec::new();
        let size = input
            .by_ref()
            .take(MAX_LINE + 1)
            .read_until(b'\n', &mut line)?;
        if size == 0 {
            break;
        }
        if size as u64 > MAX_LINE || line.last() != Some(&b'\n') {
            return Err("IPC request exceeds frame limit".into());
        }
        let req: Request = match serde_json::from_slice(&line) {
            Ok(req) => req,
            Err(_) => {
                event(
                    json!({"id":0,"type":"error","code":"invalid_request","message":"invalid native request"}),
                );
                continue;
            }
        };
        let Some(model) = models.get(&req.model) else {
            event(
                json!({"id":req.id,"type":"error","code":"model_not_found","message":"model is not registered"}),
            );
            continue;
        };
        let mut config = GenerationConfig {
            n_ctx: model.context_size,
            n_threads: std::thread::available_parallelism()
                .map(|v| v.get().min(8) as i32)
                .unwrap_or(1),
            ..GenerationConfig::default()
        };
        if let Some(v) = req.temperature {
            config.temperature = v;
        }
        if let Some(v) = req.top_p {
            config.top_p = v;
        }
        if let Some(v) = req.max_tokens {
            config.max_tokens = v;
        }
        if let Some(v) = req.timeout_ms {
            config.timeout_ms = v;
        }
        // Reserve RAM for the OS, weights and non-KV compute buffers. Existing
        // resident weights are not charged twice for repeated requests.
        let mut memory = sysinfo::System::new();
        memory.refresh_memory();
        let loading = active.as_deref() != Some(&req.model);
        let weights = if loading {
            fs::metadata(&model.path)?.len()
        } else {
            0
        };
        let budget = memory
            .available_memory()
            .saturating_sub(weights)
            .saturating_sub(1024 * 1024 * 1024)
            / 2;
        if budget < 16 * 1024 * 1024 {
            event(
                json!({"id":req.id,"type":"error","code":"resource_limit","message":"insufficient available RAM"}),
            );
            continue;
        }
        config.kv_cache_limit_bytes = budget.min(config.kv_cache_limit_bytes);
        let request = InferenceRequest::from_messages(
            req.messages
                .into_iter()
                .map(|m| ChatMessage::new(m.role, m.content))
                .collect(),
        )
        .with_config(config);
        if let Err(e) = request.validate() {
            event(
                json!({"id":req.id,"type":"error","code":"invalid_request","message":e.to_string()}),
            );
            continue;
        }
        if loading {
            cpu_only = false;
        }
        let mut spec = NativeModelSpec {
            id: req.model.clone(),
            path: model.path.clone(),
            n_gpu_layers: if cpu_only { 0 } else { model.gpu_layers },
        };
        let emitted = Arc::new(AtomicBool::new(false));
        let generate = |spec: NativeModelSpec| {
            let observed = Arc::clone(&emitted);
            worker.generate(
                spec,
                request.clone(),
                CancellationToken::new(),
                Box::new(move |chunk| {
                    if chunk.is_empty() {
                        return true;
                    }
                    observed.store(true, Ordering::Release);
                    event(json!({"id":req.id,"type":"token","text":chunk}))
                }),
            )
        };
        let mut result = generate(spec.clone());
        if spec.n_gpu_layers > 0
            && matches!(
                &result,
                Err(NativeSupervisorError::Inference(
                    native_inference::InferenceError::ModelLoad(_)
                ))
            )
            && !emitted.load(Ordering::Acquire)
        {
            spec.n_gpu_layers = 0;
            result = generate(spec);
            cpu_only = true;
        }
        match result {
            Ok(()) => {
                active = Some(req.model);
                if !event(json!({"id":req.id,"type":"done"})) {
                    break;
                }
            }
            Err(error) => {
                active = match worker.state() {
                    native_supervisor::NativeSupervisorState::Error { .. }
                        if !matches!(
                            error,
                            NativeSupervisorError::Inference(
                                native_inference::InferenceError::ModelLoad(_)
                            )
                        ) =>
                    {
                        Some(req.model)
                    }
                    _ => None,
                };
                if !event(
                    json!({"id":req.id,"type":"error","code":error_code(&error),"message":error.to_string()}),
                ) {
                    break;
                }
            }
        }
    }
    Ok(())
}
fn error_code(error: &NativeSupervisorError) -> &'static str {
    let text = error.to_string();
    if text.contains("deadline exceeded") {
        "timeout"
    } else if text.contains("KV memory budget exceeded") {
        "resource_limit"
    } else if text.contains("context limit exceeded") {
        "context_limit"
    } else {
        "generation_failed"
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("native worker: {error}");
        std::process::exit(1);
    }
}
