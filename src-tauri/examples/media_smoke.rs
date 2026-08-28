#![allow(dead_code)]

use std::{env, fs, io, path::PathBuf};

#[path = "../src/app_error.rs"]
mod app_error;
#[path = "../src/database.rs"]
mod database;
#[path = "../src/diffusion_runtime.rs"]
mod diffusion_runtime;
#[path = "../src/hardware.rs"]
mod hardware;
#[path = "../src/model_catalog.rs"]
mod model_catalog;
#[path = "../src/model_download.rs"]
mod model_download;
#[path = "../src/model_registry.rs"]
mod model_registry;
#[path = "../src/portable_root.rs"]
mod portable_root;
#[path = "../src/voice_runtime.rs"]
mod voice_runtime;

use diffusion_runtime::VideoGenerationRequest;
use hardware::HardwareProfiler;
use model_catalog::{entry_by_id, installed_file_for_pattern, ModelCatalogEntry};
use portable_root::PortableRootManager;
use reqwest::Client;

const SPEAK_MODEL_ID: &str = "kokoro-82m-onnx";
const CANVAS_MODEL_ID: &str = "sdxl-base-1";
const MOTION_MODEL_ID: &str = "wan21-t2v-13b";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Speak,
    Canvas,
    Motion,
    All,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (target, explicit_root) = parse_args()?;
    let root = match explicit_root {
        Some(path) => PortableRootManager::from_root(path),
        None => PortableRootManager::resolve()?,
    };
    root.validate_root()?;

    let output_dir = root.resolve_relative("generated/smoke")?;
    fs::create_dir_all(&output_dir)?;
    let hardware = HardwareProfiler::detect();
    let client = Client::builder().user_agent("OpenMindAI-media-smoke/2").build()?;

    println!("OpenMindAI media smoke root: {}", root.root().display());
    println!(
        "Detected: {} logical threads, {:.1} GiB RAM, {} GPU(s)",
        hardware.cpu.logical_threads,
        hardware.memory.total_bytes as f64 / 1024_f64.powi(3),
        hardware.gpus.len()
    );

    match target {
        Target::Speak => smoke_speak(&root, &output_dir).await?,
        Target::Canvas => smoke_canvas(&root, &client, &hardware, &output_dir).await?,
        Target::Motion => smoke_motion(&root, &client, &hardware, &output_dir).await?,
        Target::All => {
            smoke_speak(&root, &output_dir).await?;
            smoke_canvas(&root, &client, &hardware, &output_dir).await?;
            smoke_motion(&root, &client, &hardware, &output_dir).await?;
        }
    }

    println!("Media smoke test completed successfully.");
    Ok(())
}

fn parse_args() -> Result<(Target, Option<PathBuf>), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let target = match args.next().as_deref() {
        Some("speak") => Target::Speak,
        Some("canvas") => Target::Canvas,
        Some("motion") => Target::Motion,
        Some("all") => Target::All,
        Some("-h" | "--help") | None => {
            print_usage();
            std::process::exit(0);
        }
        Some(other) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown smoke target '{other}'"),
            )
            .into());
        }
    };

    let mut explicit_root = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => {
                let value = args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "--root requires a path")
                })?;
                explicit_root = Some(PathBuf::from(value));
            }
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown argument '{other}'"),
                )
                .into());
            }
        }
    }
    Ok((target, explicit_root))
}

fn print_usage() {
    eprintln!(
        "Usage: cargo run --manifest-path src-tauri/Cargo.toml --example media_smoke -- <speak|canvas|motion|all> [--root <OpenMindAI Root>]\n\n\
         Without --root, the harness uses OPENMINDAI_ROOT, the saved OpenMindAI installation, or the normal portable-root resolution.\n\
         'motion' can take a long time on low-VRAM hardware; 'all' runs Speak, Canvas, then Motion sequentially."
    );
}

async fn smoke_speak(
    root: &PortableRootManager,
    output_dir: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let entry = entry_by_id(SPEAK_MODEL_ID)?;
    let model = primary_model_path(root, &entry)?;
    let voice = dependency_path(root, &entry, "voice")?;
    let voices_dir = voice.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "OpenMindAI Speak voice file has no parent directory",
        )
    })?;
    let output = output_dir.join("speak-smoke.wav");

    println!("[Speak] loading real Kokoro/Candle model...");
    voice_runtime::generate_voice(
        root,
        &model,
        voices_dir,
        "OpenMindAI local voice smoke test successful.",
        &output,
    )
    .await?;
    report_output("Speak", &output, 44)?;
    Ok(())
}

async fn smoke_canvas(
    root: &PortableRootManager,
    client: &Client,
    hardware: &hardware::HardwareProfile,
    output_dir: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let entry = entry_by_id(CANVAS_MODEL_ID)?;
    let model = primary_model_path(root, &entry)?;
    let output = output_dir.join("canvas-smoke.png");

    println!("[Canvas] running real stable-diffusion.cpp image inference...");
    diffusion_runtime::generate_image(
        root,
        client,
        hardware,
        &model,
        "A simple red circle centered on a clean white background, minimal composition.",
        &output,
    )
    .await?;
    report_output("Canvas", &output, 8)?;
    Ok(())
}

async fn smoke_motion(
    root: &PortableRootManager,
    client: &Client,
    hardware: &hardware::HardwareProfile,
    output_dir: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let entry = entry_by_id(MOTION_MODEL_ID)?;
    let diffusion_model = primary_model_path(root, &entry)?;
    let vae = dependency_path(root, &entry, "vae")?;
    let text_encoder = dependency_path(root, &entry, "text-encoder")?;
    let output = output_dir.join("motion-smoke.webm");

    println!("[Motion] running real Wan/stable-diffusion.cpp video inference...");
    diffusion_runtime::generate_video(
        root,
        client,
        hardware,
        VideoGenerationRequest {
            diffusion_model_path: &diffusion_model,
            vae_path: &vae,
            text_encoder_path: &text_encoder,
            prompt: "A simple white sphere slowly rotating on a dark background, static camera, smooth motion.",
            output_path: &output,
        },
    )
    .await?;
    report_output("Motion", &output, 4)?;
    Ok(())
}

fn primary_model_path(
    root: &PortableRootManager,
    entry: &ModelCatalogEntry,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let download = entry.download.as_ref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} has no download metadata", entry.name),
        )
    })?;
    installed_file_for_pattern(root, &download.destination_dir, &download.filename_pattern)
        .ok_or_else(|| missing_model_error(&entry.name).into())
}

fn dependency_path(
    root: &PortableRootManager,
    entry: &ModelCatalogEntry,
    role: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let download = entry.download.as_ref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} has no download metadata", entry.name),
        )
    })?;
    let dependency = download
        .dependencies
        .iter()
        .find(|dependency| dependency.role == role)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{} catalog entry has no '{role}' dependency", entry.name),
            )
        })?;
    installed_file_for_pattern(root, &download.destination_dir, &dependency.filename_pattern)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "{} dependency '{}' is not installed; validate or re-download it from Settings > Models",
                    entry.name, role
                ),
            )
            .into()
        })
}

fn missing_model_error(name: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{name} is not installed; download it from Settings > Models first"),
    )
}

fn report_output(
    label: &str,
    path: &std::path::Path,
    minimum_bytes: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let size = fs::metadata(path)?.len();
    if size <= minimum_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{label} output is unexpectedly small: {size} bytes"),
        )
        .into());
    }
    println!("[{label}] PASS: {} ({size} bytes)", path.display());
    Ok(())
}
