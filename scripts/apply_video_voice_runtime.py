from pathlib import Path
import json


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    if old not in text:
        raise SystemExit(f"anchor not found in {path}: {old[:120]!r}")
    file.write_text(text.replace(old, new, 1), encoding="utf-8")


# Native local Kokoro TTS runtime. Its default feature bundles the phonemizer,
# so end users do not need Python or a separately-installed eSpeak runtime.
replace_once(
    "src-tauri/Cargo.toml",
    'futures-util = "0.3"\n',
    'futures-util = "0.3"\nkokoro-en = "0.1.5"\n',
)

# Upgrade the bundled model catalog to production media packages.
catalog_path = Path("src-tauri/model-catalog.json")
catalog = json.loads(catalog_path.read_text(encoding="utf-8"))
catalog["catalogVersion"] = max(int(catalog.get("catalogVersion", 0)), 4)
models = catalog["models"]

speak = next(model for model in models if model["id"] == "kokoro-82m-onnx")
speak.update(
    {
        "version": "1.0",
        "family": "kokoro",
        "runtime": "kokoro-en",
        "quantization": "Q8",
        "sizeBytes": 94000000,
        "minRamBytes": 4294967296,
        "minVramBytes": None,
        "description": "Native local Kokoro speech synthesis with bundled phonemization and verified voice weights.",
        "download": {
            "strategy": "singleFile",
            "filenamePattern": "*model_quantized.onnx",
            "destinationDir": "models/audio/kokoro",
            "format": "onnx",
            "dependencies": [
                {
                    "role": "voice",
                    "filenamePattern": "*af_heart.bin",
                    "format": "bin",
                    "required": True,
                }
            ],
        },
    }
)

# Retire the old ComfyUI-only LTX placeholder. OpenMindAI Motion now uses the
# Wan2.1 1.3B T2V path supported directly by the same stable-diffusion.cpp
# runtime already used for Canvas image generation.
models[:] = [model for model in models if model["id"] != "ltx-video"]
motion = next(model for model in models if model["id"] == "wan21-t2v-13b")
motion.update(
    {
        "name": "OpenMindAI Motion",
        "version": "2.1",
        "family": "wan",
        "kind": "video",
        "runtime": "stable-diffusion.cpp",
        "repo": "Comfy-Org/Wan_2.1_ComfyUI_repackaged",
        "quantization": "FP16+Q5_K_M",
        "required": False,
        "capabilities": ["text-to-video", "video"],
        "sizeBytes": 7250000000,
        "minRamBytes": 17179869184,
        "minVramBytes": 6442450944,
        "license": "apache-2.0",
        "description": "Local Wan2.1 1.3B text-to-video generation through stable-diffusion.cpp with WebM output and memory-aware offload.",
        "download": {
            "strategy": "singleFile",
            "filenamePattern": "*wan2.1_t2v_1.3B_fp16.safetensors",
            "destinationDir": "models/video/wan2.1-t2v-1.3b",
            "format": "safetensors",
            "dependencies": [
                {
                    "role": "vae",
                    "filenamePattern": "*wan_2.1_vae.safetensors",
                    "format": "safetensors",
                    "required": True,
                },
                {
                    "role": "text-encoder",
                    "repo": "city96/umt5-xxl-encoder-gguf",
                    "filenamePattern": "*umt5-xxl-encoder-Q5_K_M.gguf",
                    "format": "gguf",
                    "required": True,
                },
            ],
        },
    }
)
catalog_path.write_text(json.dumps(catalog, indent=2) + "\n", encoding="utf-8")

# Catalog dependencies can now live in a different Hugging Face repository
# (Wan's UMT5 GGUF encoder is intentionally distributed separately).
replace_once(
    "src-tauri/src/model_catalog.rs",
    '''pub struct ModelCatalogDependency {\n    pub role: String,\n    pub filename_pattern: String,\n    pub format: String,\n    #[serde(default = "default_required_dependency")]\n    pub required: bool,\n}''',
    '''pub struct ModelCatalogDependency {\n    pub role: String,\n    #[serde(default)]\n    pub repo: Option<String>,\n    pub filename_pattern: String,\n    pub format: String,\n    #[serde(default = "default_required_dependency")]\n    pub required: bool,\n}''',
)
replace_once(
    "src-tauri/src/model_catalog.rs",
    '''fn find_downloaded_file(\n    root: &PortableRootManager,\n    download: &ModelCatalogDownload,\n) -> Option<String> {\n    let dir = root.resolve_relative(&download.destination_dir).ok()?;\n    if !dir.exists() {\n        return None;\n    }\n    find_file_matching(&dir, &download.filename_pattern)\n        .and_then(|path| path.strip_prefix(root.root()).ok().map(Path::to_path_buf))\n        .map(|path| path.to_string_lossy().replace('\\\\', "/"))\n}\n''',
    '''fn find_downloaded_file(\n    root: &PortableRootManager,\n    download: &ModelCatalogDownload,\n) -> Option<String> {\n    let dir = root.resolve_relative(&download.destination_dir).ok()?;\n    if !dir.exists() {\n        return None;\n    }\n    find_file_matching(&dir, &download.filename_pattern)\n        .and_then(|path| path.strip_prefix(root.root()).ok().map(Path::to_path_buf))\n        .map(|path| path.to_string_lossy().replace('\\\\', "/"))\n}\n\npub(crate) fn installed_file_for_pattern(\n    root: &PortableRootManager,\n    destination_dir: &str,\n    pattern: &str,\n) -> Option<PathBuf> {\n    let dir = root.resolve_relative(destination_dir).ok()?;\n    if !dir.exists() {\n        return None;\n    }\n    find_file_matching(&dir, pattern)\n}\n''',
)
replace_once(
    "src-tauri/src/model_catalog.rs",
    '''        assert!(lens\n            .download\n            .as_ref()\n            .unwrap()\n            .dependencies\n            .iter()\n            .any(|dependency| dependency.role == "mmproj" && dependency.required));''',
    '''        assert!(lens\n            .download\n            .as_ref()\n            .unwrap()\n            .dependencies\n            .iter()\n            .any(|dependency| dependency.role == "mmproj" && dependency.required));\n\n        let speak = catalog\n            .models\n            .iter()\n            .find(|entry| entry.id == "kokoro-82m-onnx")\n            .unwrap();\n        assert_eq!(speak.runtime, "kokoro-en");\n        assert!(speak\n            .download\n            .as_ref()\n            .unwrap()\n            .dependencies\n            .iter()\n            .any(|dependency| dependency.role == "voice" && dependency.required));\n\n        let motion = catalog\n            .models\n            .iter()\n            .find(|entry| entry.id == "wan21-t2v-13b")\n            .unwrap();\n        assert_eq!(motion.runtime, "stable-diffusion.cpp");\n        assert!(motion\n            .download\n            .as_ref()\n            .unwrap()\n            .dependencies\n            .iter()\n            .any(|dependency| dependency.role == "text-encoder"\n                && dependency.repo.as_deref() == Some("city96/umt5-xxl-encoder-gguf")));''',
)

# Download nested Hugging Face paths safely: installed paths retain their
# repository layout while partial-download filenames are flattened into temp/.
replace_once(
    "src-tauri/src/model_download.rs",
    '''        let final_path = model_dir.join(&filename);\n        let part_path = temp_dir.join(format!("{filename}.part"));\n        ensure_contained(self.root.root(), &final_path)?;\n        ensure_contained(self.root.root(), &part_path)?;''',
    '''        let final_path = model_dir.join(&filename);\n        if let Some(parent) = final_path.parent() {\n            fs::create_dir_all(parent)?;\n            ensure_contained(self.root.root(), parent)?;\n        }\n        let part_path = temp_dir.join(safe_part_filename(&filename));\n        ensure_contained(self.root.root(), &final_path)?;\n        ensure_contained(self.root.root(), &part_path)?;''',
)
replace_once(
    "src-tauri/src/model_download.rs",
    '''fn select_sibling<'a>(\n    siblings: &'a [HuggingFaceSibling],\n    download: &ModelCatalogDownload,\n) -> Option<&'a HuggingFaceSibling> {''',
    '''pub(crate) fn safe_part_filename(filename: &str) -> String {\n    let flattened = filename\n        .chars()\n        .map(|character| if matches!(character, '/' | '\\\\') { '_' } else { character })\n        .collect::<String>();\n    format!("{flattened}.part")\n}\n\nfn select_sibling<'a>(\n    siblings: &'a [HuggingFaceSibling],\n    download: &ModelCatalogDownload,\n) -> Option<&'a HuggingFaceSibling> {''',
)
replace_once(
    "src-tauri/src/model_download.rs",
    '''    #[test]\n    fn verify_existing_file_accepts_matching_checksum() {''',
    '''    #[test]\n    fn nested_hugging_face_paths_get_safe_partial_names() {\n        assert_eq!(\n            safe_part_filename("split_files/diffusion_models/model.safetensors"),\n            "split_files_diffusion_models_model.safetensors.part"\n        );\n        assert_eq!(safe_part_filename("onnx\\\\model.onnx"), "onnx_model.onnx.part");\n    }\n\n    #[test]\n    fn verify_existing_file_accepts_matching_checksum() {''',
)

# Resolve each dependency from its own repo when configured, preserve checksum
# verification, and safely handle nested dependency filenames.
replace_once(
    "src-tauri/src/model_package.rs",
    '''    model_download::{ensure_contained, sha256_file, validate_free_space, VerificationState},''',
    '''    model_download::{\n        ensure_contained, safe_part_filename, sha256_file, validate_free_space, VerificationState,\n    },''',
)
replace_once(
    "src-tauri/src/model_package.rs",
    '''    let api_url = format!(\n        "https://huggingface.co/api/models/{}?blobs=true",\n        entry.repo\n    );\n    let model: HuggingFaceModel = client\n        .get(api_url)\n        .send()\n        .await\n        .map_err(|error| AppError::ModelDownloadFailed(error.to_string()))?\n        .error_for_status()\n        .map_err(|error| AppError::ModelDownloadFailed(error.to_string()))?\n        .json()\n        .await\n        .map_err(|error| AppError::ModelDownloadFailed(error.to_string()))?;\n\n    let mut files = Vec::new();\n    for dependency in &download.dependencies {\n        if cancellation.is_cancelled() {\n            return Err(AppError::InferenceCancelled("download stopped".to_string()));\n        }\n        match resolve_dependency(&entry.repo, &model, dependency) {''',
    '''    let mut files = Vec::new();\n    for dependency in &download.dependencies {\n        if cancellation.is_cancelled() {\n            return Err(AppError::InferenceCancelled("download stopped".to_string()));\n        }\n        let repo = dependency.repo.as_deref().unwrap_or(&entry.repo);\n        let model = match fetch_model_metadata(client, repo).await {\n            Ok(model) => model,\n            Err(error) if !dependency.required => {\n                tracing::warn!(role = %dependency.role, %repo, %error, "optional model package repository unavailable");\n                continue;\n            }\n            Err(error) => return Err(error),\n        };\n        match resolve_dependency(repo, &model, dependency) {''',
)
replace_once(
    "src-tauri/src/model_package.rs",
    '''fn resolve_dependency(\n    repo: &str,''',
    '''async fn fetch_model_metadata(client: &Client, repo: &str) -> Result<HuggingFaceModel, AppError> {\n    let api_url = format!("https://huggingface.co/api/models/{repo}?blobs=true");\n    client\n        .get(api_url)\n        .send()\n        .await\n        .map_err(|error| AppError::ModelDownloadFailed(error.to_string()))?\n        .error_for_status()\n        .map_err(|error| AppError::ModelDownloadFailed(error.to_string()))?\n        .json()\n        .await\n        .map_err(|error| AppError::ModelDownloadFailed(error.to_string()))\n}\n\nfn resolve_dependency(\n    repo: &str,''',
)
replace_once(
    "src-tauri/src/model_package.rs",
    '''    let final_path = model_dir.join(&dependency.filename);\n    let temp_dir = root.resolve_relative("temp/downloads")?;\n    fs::create_dir_all(&temp_dir)?;\n    let part_path = temp_dir.join(format!("{}.part", dependency.filename));\n    ensure_contained(root.root(), &final_path)?;\n    ensure_contained(root.root(), &part_path)?;''',
    '''    let final_path = model_dir.join(&dependency.filename);\n    if let Some(parent) = final_path.parent() {\n        fs::create_dir_all(parent)?;\n        ensure_contained(root.root(), parent)?;\n    }\n    let temp_dir = root.resolve_relative("temp/downloads")?;\n    fs::create_dir_all(&temp_dir)?;\n    let part_path = temp_dir.join(safe_part_filename(&dependency.filename));\n    ensure_contained(root.root(), &final_path)?;\n    ensure_contained(root.root(), &part_path)?;''',
)

# WebM is the native video container emitted by the prebuilt sd-cli runtime.
replace_once(
    "src-tauri/src/artifacts.rs",
    '        "video" => "video/mp4",',
    '        "video" => "video/webm",',
)
replace_once(
    "src-tauri/src/artifacts.rs",
    '        "video" => "mp4",',
    '        "video" => "webm",',
)

# Extend the existing verified stable-diffusion.cpp runtime with Wan video
# generation. Runtime download/checksum/extraction is shared with images.
replace_once(
    "src-tauri/src/diffusion_runtime.rs",
    '''const GENERATION_TIMEOUT: Duration = Duration::from_secs(20 * 60);\nconst PNG_SIGNATURE: &[u8; 8] = b"\\x89PNG\\r\\n\\x1a\\n";''',
    '''const GENERATION_TIMEOUT: Duration = Duration::from_secs(20 * 60);\nconst VIDEO_GENERATION_TIMEOUT: Duration = Duration::from_secs(90 * 60);\nconst PNG_SIGNATURE: &[u8; 8] = b"\\x89PNG\\r\\n\\x1a\\n";\nconst WEBM_SIGNATURE: &[u8; 4] = b"\\x1a\\x45\\xdf\\xa3";''',
)
replace_once(
    "src-tauri/src/diffusion_runtime.rs",
    '''struct RenderProfile {\n    width: u32,\n    height: u32,\n    steps: u32,\n    cfg_scale: u32,\n    clip_on_cpu: bool,\n    vae_on_cpu: bool,\n    offload_to_cpu: bool,\n}\n''',
    '''struct RenderProfile {\n    width: u32,\n    height: u32,\n    steps: u32,\n    cfg_scale: u32,\n    clip_on_cpu: bool,\n    vae_on_cpu: bool,\n    offload_to_cpu: bool,\n}\n\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\nstruct VideoRenderProfile {\n    width: u32,\n    height: u32,\n    frames: u32,\n    fps: u32,\n    offload_to_cpu: bool,\n    clip_on_cpu: bool,\n    vae_on_cpu: bool,\n}\n''',
)
video_function = r'''pub async fn generate_video(
    root: &PortableRootManager,
    client: &Client,
    hardware: &HardwareProfile,
    diffusion_model_path: &Path,
    vae_path: &Path,
    text_encoder_path: &Path,
    prompt: &str,
    output_path: &Path,
) -> Result<(), AppError> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err(AppError::ArtifactGenerationFailed(
            "video prompt cannot be empty".to_string(),
        ));
    }
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(AppError::ArtifactGenerationFailed(format!(
            "video prompt is too long; maximum is {MAX_PROMPT_CHARS} characters"
        )));
    }

    let diffusion_model_path =
        canonical_file_under_root(root, diffusion_model_path, "video diffusion model")?;
    let vae_path = canonical_file_under_root(root, vae_path, "video VAE")?;
    let text_encoder_path =
        canonical_file_under_root(root, text_encoder_path, "video text encoder")?;
    let output_parent = output_path.parent().ok_or_else(|| {
        AppError::ArtifactGenerationFailed("video output path has no parent".to_string())
    })?;
    fs::create_dir_all(output_parent)?;
    ensure_contained(root.root(), output_path).map_err(|error| {
        AppError::ArtifactGenerationFailed(format!("video output path rejected: {error}"))
    })?;

    let runtime = ensure_runtime(root, client, hardware).await?;
    let cli_path = root.resolve_relative(&runtime.cli_path)?;
    let profile = video_render_profile(hardware, &runtime.backend);
    if output_path.exists() {
        fs::remove_file(output_path)?;
    }

    let mut command = Command::new(&cli_path);
    command
        .arg("-M")
        .arg("vid_gen")
        .arg("--diffusion-model")
        .arg(&diffusion_model_path)
        .arg("--vae")
        .arg(&vae_path)
        .arg("--t5xxl")
        .arg(&text_encoder_path)
        .arg("-p")
        .arg(prompt)
        .arg("-n")
        .arg("worst quality, low quality, blurry, distorted, artifacts, text, watermark, flicker, jitter, temporal inconsistency")
        .arg("-o")
        .arg(output_path)
        .arg("--cfg-scale")
        .arg("6.0")
        .arg("--sampling-method")
        .arg("euler")
        .arg("-W")
        .arg(profile.width.to_string())
        .arg("-H")
        .arg(profile.height.to_string())
        .arg("--video-frames")
        .arg(profile.frames.to_string())
        .arg("--fps")
        .arg(profile.fps.to_string())
        .arg("--flow-shift")
        .arg("3.0")
        .arg("--diffusion-fa")
        .arg("--mmap")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    if profile.offload_to_cpu {
        command.arg("--offload-to-cpu");
    }
    if profile.clip_on_cpu {
        command.arg("--clip-on-cpu");
    }
    if profile.vae_on_cpu {
        command.arg("--vae-on-cpu");
    }
    if let Some(parent) = cli_path.parent() {
        command.current_dir(parent);
    }
    hide_console_window(&mut command);

    tracing::info!(
        backend = ?runtime.backend,
        width = profile.width,
        height = profile.height,
        frames = profile.frames,
        fps = profile.fps,
        model = %diffusion_model_path.display(),
        "starting local Wan video generation"
    );

    let output = timeout(VIDEO_GENERATION_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            AppError::ArtifactGenerationFailed(
                "local video generation timed out after 90 minutes".to_string(),
            )
        })?
        .map_err(|error| {
            AppError::ArtifactGenerationFailed(format!(
                "could not start stable-diffusion.cpp video runtime: {error}"
            ))
        })?;

    if !output.status.success() {
        let detail = process_error_detail(&output.stdout, &output.stderr);
        return Err(AppError::ArtifactGenerationFailed(format!(
            "stable-diffusion.cpp video generation exited with {}: {detail}",
            output.status
        )));
    }

    validate_webm(output_path)?;
    tracing::info!(path = %output_path.display(), "local Wan WebM video generated");
    Ok(())
}

'''
replace_once(
    "src-tauri/src/diffusion_runtime.rs",
    "async fn ensure_runtime(\n",
    video_function + "async fn ensure_runtime(\n",
)
video_profile = r'''fn video_render_profile(hardware: &HardwareProfile, backend: &BackendKind) -> VideoRenderProfile {
    if *backend == BackendKind::Cpu {
        return VideoRenderProfile {
            width: 384,
            height: 224,
            frames: 17,
            fps: 8,
            offload_to_cpu: false,
            clip_on_cpu: true,
            vae_on_cpu: true,
        };
    }

    let max_vram = hardware
        .gpus
        .iter()
        .filter(|gpu| !gpu.is_software)
        .filter_map(|gpu| gpu.dedicated_vram_bytes)
        .max()
        .unwrap_or(0);
    let gib = 1024_u64.pow(3);
    if max_vram <= 8 * gib {
        VideoRenderProfile {
            width: 512,
            height: 288,
            frames: 17,
            fps: 8,
            offload_to_cpu: true,
            clip_on_cpu: true,
            vae_on_cpu: true,
        }
    } else if max_vram <= 12 * gib {
        VideoRenderProfile {
            width: 640,
            height: 368,
            frames: 25,
            fps: 12,
            offload_to_cpu: true,
            clip_on_cpu: true,
            vae_on_cpu: true,
        }
    } else {
        VideoRenderProfile {
            width: 832,
            height: 480,
            frames: 33,
            fps: 16,
            offload_to_cpu: false,
            clip_on_cpu: false,
            vae_on_cpu: false,
        }
    }
}

'''
replace_once(
    "src-tauri/src/diffusion_runtime.rs",
    "fn render_profile(hardware: &HardwareProfile, backend: &BackendKind) -> RenderProfile {\n",
    video_profile + "fn render_profile(hardware: &HardwareProfile, backend: &BackendKind) -> RenderProfile {\n",
)
webm_validation = r'''fn validate_webm(path: &Path) -> Result<(), AppError> {
    let metadata = fs::metadata(path).map_err(|error| {
        AppError::ArtifactGenerationFailed(format!("video output was not created: {error}"))
    })?;
    if metadata.len() < 4096 {
        return Err(AppError::ArtifactGenerationFailed(
            "video output is implausibly small".to_string(),
        ));
    }
    let mut file = fs::File::open(path)?;
    let mut signature = [0_u8; 4];
    file.read_exact(&mut signature)?;
    if &signature != WEBM_SIGNATURE {
        return Err(AppError::ArtifactGenerationFailed(
            "video runtime did not produce a valid WebM/EBML container".to_string(),
        ));
    }
    Ok(())
}

'''
replace_once(
    "src-tauri/src/diffusion_runtime.rs",
    "fn validate_png(path: &Path) -> Result<(), AppError> {\n",
    webm_validation + "fn validate_png(path: &Path) -> Result<(), AppError> {\n",
)
with Path("src-tauri/src/diffusion_runtime.rs").open("a", encoding="utf-8") as file:
    file.write(
        r'''

#[cfg(test)]
mod video_runtime_tests {
    use super::*;

    #[test]
    fn webm_validator_accepts_ebml_container_signature() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("clip.webm");
        let mut bytes = WEBM_SIGNATURE.to_vec();
        bytes.resize(8192, 0);
        fs::write(&path, bytes).unwrap();
        validate_webm(&path).unwrap();
    }

    #[test]
    fn webm_validator_rejects_wrong_container() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("clip.webm");
        fs::write(&path, vec![0_u8; 8192]).unwrap();
        assert!(validate_webm(&path).is_err());
    }
}
'''
    )

# Native TTS module: Kokoro inference -> 24 kHz mono PCM WAV -> strict output validation.
Path("src-tauri/src/voice_runtime.rs").write_text(
    r'''use std::{fs, io::{Read, Write}, path::{Path, PathBuf}};

use kokoro_en::{KokoroTts, Voice};

use crate::{
    app_error::AppError,
    model_download::ensure_contained,
    portable_root::PortableRootManager,
};

const SAMPLE_RATE: u32 = 24_000;
const DEFAULT_VOICE: &str = "af_heart";
const MAX_TTS_CHARS: usize = 12_000;
const WAV_HEADER_BYTES: u64 = 44;

pub async fn generate_voice(
    root: &PortableRootManager,
    model_path: &Path,
    voices_dir: &Path,
    text: &str,
    output_path: &Path,
) -> Result<(), AppError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::ArtifactGenerationFailed(
            "voice narration cannot be empty".to_string(),
        ));
    }
    if text.chars().count() > MAX_TTS_CHARS {
        return Err(AppError::ArtifactGenerationFailed(format!(
            "voice narration is too long; maximum is {MAX_TTS_CHARS} characters"
        )));
    }

    let model_path = canonical_file_under_root(root, model_path, "Kokoro model")?;
    let voices_dir = canonical_dir_under_root(root, voices_dir, "Kokoro voices")?;
    if !voices_dir.join(format!("{DEFAULT_VOICE}.bin")).is_file() {
        return Err(AppError::ArtifactGenerationFailed(format!(
            "required Kokoro voice {DEFAULT_VOICE} is missing"
        )));
    }

    let output_parent = output_path.parent().ok_or_else(|| {
        AppError::ArtifactGenerationFailed("voice output path has no parent".to_string())
    })?;
    fs::create_dir_all(output_parent)?;
    ensure_contained(root.root(), output_path).map_err(|error| {
        AppError::ArtifactGenerationFailed(format!("voice output path rejected: {error}"))
    })?;
    if output_path.exists() {
        fs::remove_file(output_path)?;
    }

    tracing::info!(voice = DEFAULT_VOICE, "loading local Kokoro voice runtime");
    let tts = KokoroTts::new(&model_path, &voices_dir)
        .await
        .map_err(|error| {
            AppError::ArtifactGenerationFailed(format!("could not load Kokoro TTS: {error}"))
        })?;
    let (samples, elapsed) = tts
        .synth(text, Voice::new(DEFAULT_VOICE))
        .await
        .map_err(|error| {
            AppError::ArtifactGenerationFailed(format!("Kokoro speech synthesis failed: {error}"))
        })?;
    if samples.is_empty() {
        return Err(AppError::ArtifactGenerationFailed(
            "Kokoro returned no audio samples".to_string(),
        ));
    }

    write_pcm16_wav(output_path, &samples)?;
    validate_wav(output_path)?;
    tracing::info!(
        path = %output_path.display(),
        samples = samples.len(),
        elapsed_ms = elapsed.as_millis(),
        "local Kokoro WAV generated"
    );
    Ok(())
}

fn canonical_file_under_root(
    root: &PortableRootManager,
    path: &Path,
    label: &str,
) -> Result<PathBuf, AppError> {
    let canonical_root = fs::canonicalize(root.root())?;
    let canonical = fs::canonicalize(path).map_err(|error| {
        AppError::ArtifactGenerationFailed(format!("{label} is unavailable: {error}"))
    })?;
    if !canonical.is_file() || !canonical.starts_with(&canonical_root) {
        return Err(AppError::ArtifactGenerationFailed(format!(
            "{label} is outside OpenMindAI Root or is not a file"
        )));
    }
    Ok(canonical)
}

fn canonical_dir_under_root(
    root: &PortableRootManager,
    path: &Path,
    label: &str,
) -> Result<PathBuf, AppError> {
    let canonical_root = fs::canonicalize(root.root())?;
    let canonical = fs::canonicalize(path).map_err(|error| {
        AppError::ArtifactGenerationFailed(format!("{label} are unavailable: {error}"))
    })?;
    if !canonical.is_dir() || !canonical.starts_with(&canonical_root) {
        return Err(AppError::ArtifactGenerationFailed(format!(
            "{label} are outside OpenMindAI Root or are not a directory"
        )));
    }
    Ok(canonical)
}

fn write_pcm16_wav(path: &Path, samples: &[f32]) -> Result<(), AppError> {
    let data_bytes = samples
        .len()
        .checked_mul(2)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| AppError::ArtifactGenerationFailed("voice output is too large".to_string()))?;
    let riff_size = 36_u32
        .checked_add(data_bytes)
        .ok_or_else(|| AppError::ArtifactGenerationFailed("voice output is too large".to_string()))?;

    let mut file = fs::File::create(path)?;
    file.write_all(b"RIFF")?;
    file.write_all(&riff_size.to_le_bytes())?;
    file.write_all(b"WAVE")?;
    file.write_all(b"fmt ")?;
    file.write_all(&16_u32.to_le_bytes())?;
    file.write_all(&1_u16.to_le_bytes())?;
    file.write_all(&1_u16.to_le_bytes())?;
    file.write_all(&SAMPLE_RATE.to_le_bytes())?;
    file.write_all(&(SAMPLE_RATE * 2).to_le_bytes())?;
    file.write_all(&2_u16.to_le_bytes())?;
    file.write_all(&16_u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_bytes.to_le_bytes())?;
    for sample in samples {
        let pcm = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        file.write_all(&pcm.to_le_bytes())?;
    }
    file.flush()?;
    Ok(())
}

fn validate_wav(path: &Path) -> Result<(), AppError> {
    let metadata = fs::metadata(path).map_err(|error| {
        AppError::ArtifactGenerationFailed(format!("voice output was not created: {error}"))
    })?;
    if metadata.len() <= WAV_HEADER_BYTES {
        return Err(AppError::ArtifactGenerationFailed(
            "voice output contains no PCM audio".to_string(),
        ));
    }
    let mut file = fs::File::open(path)?;
    let mut header = [0_u8; 44];
    file.read_exact(&mut header)?;
    if &header[0..4] != b"RIFF"
        || &header[8..12] != b"WAVE"
        || &header[12..16] != b"fmt "
        || &header[36..40] != b"data"
    {
        return Err(AppError::ArtifactGenerationFailed(
            "Kokoro runtime did not produce a valid WAV container".to_string(),
        ));
    }
    if u32::from_le_bytes(header[24..28].try_into().unwrap()) != SAMPLE_RATE {
        return Err(AppError::ArtifactGenerationFailed(
            "Kokoro WAV has an unexpected sample rate".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_validates_pcm16_wav() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("voice.wav");
        let samples = (0..2400)
            .map(|index| ((index as f32 / 20.0).sin() * 0.25).clamp(-1.0, 1.0))
            .collect::<Vec<_>>();
        write_pcm16_wav(&path, &samples).unwrap();
        validate_wav(&path).unwrap();
        assert_eq!(fs::metadata(path).unwrap().len(), 44 + samples.len() as u64 * 2);
    }

    #[test]
    fn rejects_non_wav_output() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("voice.wav");
        fs::write(&path, vec![0_u8; 128]).unwrap();
        assert!(validate_wav(&path).is_err());
    }
}
''',
    encoding="utf-8",
)

# Wire both runtime paths into the Tauri generation command.
replace_once(
    "src-tauri/src/lib.rs",
    "mod storage;\n",
    "mod storage;\nmod voice_runtime;\n",
)
replace_once(
    "src-tauri/src/lib.rs",
    '''        "video" => {\n            let installed = installed_catalog_entry_for_kind(state, "video")?;\n            let Some(model) = installed else {\n                return Err(app_error::AppError::ArtifactGenerationFailed(\n                    "OpenMindAI Motion model download required. Open Settings > Models and download the recommended model first."\n                        .to_string(),\n                ));\n            };\n            Err(app_error::AppError::ArtifactGenerationFailed(format!(\n                "{} is downloaded, but the local video runner is not connected yet. Install the OpenMindAI Motion runtime connector to render MP4 output.",\n                model.entry.name\n            )))\n        }\n        "voice" => {\n            let installed = installed_catalog_entry_for_kind(state, "text-to-speech")?;\n            let Some(model) = installed else {\n                return Err(app_error::AppError::ArtifactGenerationFailed(\n                    "OpenMindAI Speak model download required. Open Settings > Models and download the recommended model first."\n                        .to_string(),\n                ));\n            };\n            Err(app_error::AppError::ArtifactGenerationFailed(format!(\n                "{} is downloaded, but the local voice runner is not connected yet. Install the OpenMindAI Speak runtime connector to render WAV output.",\n                model.entry.name\n            )))\n        }''',
    '''        "video" => {\n            let model = installed_catalog_entry_by_id(state, "wan21-t2v-13b")?.ok_or_else(|| {\n                app_error::AppError::ArtifactGenerationFailed(\n                    "OpenMindAI Motion model package is required. Open Settings > Models and download OpenMindAI Motion first."\n                        .to_string(),\n                )\n            })?;\n            let relative_model_path = model.installed_path.as_deref().ok_or_else(|| {\n                app_error::AppError::ArtifactGenerationFailed(\n                    "OpenMindAI Motion model path is unavailable".to_string(),\n                )\n            })?;\n            let model_path = state.root.resolve_relative(relative_model_path)?;\n            let download = model.entry.download.as_ref().ok_or_else(|| {\n                app_error::AppError::ArtifactGenerationFailed(\n                    "OpenMindAI Motion package metadata is unavailable".to_string(),\n                )\n            })?;\n            let vae_dependency = download\n                .dependencies\n                .iter()\n                .find(|dependency| dependency.role == "vae")\n                .ok_or_else(|| app_error::AppError::ArtifactGenerationFailed("OpenMindAI Motion VAE metadata is missing".to_string()))?;\n            let text_dependency = download\n                .dependencies\n                .iter()\n                .find(|dependency| dependency.role == "text-encoder")\n                .ok_or_else(|| app_error::AppError::ArtifactGenerationFailed("OpenMindAI Motion text encoder metadata is missing".to_string()))?;\n            let vae_path = model_catalog::installed_file_for_pattern(\n                &state.root,\n                &download.destination_dir,\n                &vae_dependency.filename_pattern,\n            )\n            .ok_or_else(|| app_error::AppError::ArtifactGenerationFailed("OpenMindAI Motion VAE is missing; validate or re-download the model package".to_string()))?;\n            let text_encoder_path = model_catalog::installed_file_for_pattern(\n                &state.root,\n                &download.destination_dir,\n                &text_dependency.filename_pattern,\n            )\n            .ok_or_else(|| app_error::AppError::ArtifactGenerationFailed("OpenMindAI Motion text encoder is missing; validate or re-download the model package".to_string()))?;\n            let hardware = HardwareProfiler::detect();\n            diffusion_runtime::generate_video(\n                &state.root,\n                &state.http,\n                &hardware,\n                &model_path,\n                &vae_path,\n                &text_encoder_path,\n                prompt,\n                path,\n            )\n            .await\n        }\n        "voice" => {\n            let model = installed_catalog_entry_by_id(state, "kokoro-82m-onnx")?.ok_or_else(|| {\n                app_error::AppError::ArtifactGenerationFailed(\n                    "OpenMindAI Speak model package is required. Open Settings > Models and download OpenMindAI Speak first."\n                        .to_string(),\n                )\n            })?;\n            let relative_model_path = model.installed_path.as_deref().ok_or_else(|| {\n                app_error::AppError::ArtifactGenerationFailed(\n                    "OpenMindAI Speak model path is unavailable".to_string(),\n                )\n            })?;\n            let model_path = state.root.resolve_relative(relative_model_path)?;\n            let download = model.entry.download.as_ref().ok_or_else(|| {\n                app_error::AppError::ArtifactGenerationFailed(\n                    "OpenMindAI Speak package metadata is unavailable".to_string(),\n                )\n            })?;\n            let voice_dependency = download\n                .dependencies\n                .iter()\n                .find(|dependency| dependency.role == "voice")\n                .ok_or_else(|| app_error::AppError::ArtifactGenerationFailed("OpenMindAI Speak voice metadata is missing".to_string()))?;\n            let voice_path = model_catalog::installed_file_for_pattern(\n                &state.root,\n                &download.destination_dir,\n                &voice_dependency.filename_pattern,\n            )\n            .ok_or_else(|| app_error::AppError::ArtifactGenerationFailed("OpenMindAI Speak voice weights are missing; validate or re-download the model package".to_string()))?;\n            let voices_dir = voice_path.parent().ok_or_else(|| {\n                app_error::AppError::ArtifactGenerationFailed(\n                    "OpenMindAI Speak voice directory is unavailable".to_string(),\n                )\n            })?;\n            voice_runtime::generate_voice(&state.root, &model_path, voices_dir, prompt, path).await\n        }''',
)
replace_once(
    "src-tauri/src/lib.rs",
    '''fn installed_catalog_entry_for_kind(\n    state: &AppState,\n    kind: &str,\n) -> Result<Option<model_catalog::ModelCatalogStatus>, app_error::AppError> {\n    let db = state\n        .database\n        .lock()\n        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;\n    let installed = ModelRegistry::new(&db, &state.root).discover_gguf_models()?;\n    drop(db);\n    let hardware = HardwareProfiler::detect();\n    Ok(\n        model_catalog::check_model_updates(&installed, &hardware, &state.root)?\n            .entries\n            .into_iter()\n            .find(|item| item.entry.kind == kind && item.installed),\n    )\n}\n\n''',
    "",
)
# Return the post-stream database snapshot instead of the stale empty assistant,
# allowing media artifact creation to use the model-refined generation prompt/script.
replace_once(
    "src-tauri/src/lib.rs",
    '''    run_streaming_completion(\n        &app,\n        &state,\n        &conversation_id,\n        &model,\n        &assistant,\n        &mode,\n        &media,\n    )\n    .await?;\n    Ok(assistant)\n}''',
    '''    run_streaming_completion(\n        &app,\n        &state,\n        &conversation_id,\n        &model,\n        &assistant,\n        &mode,\n        &media,\n    )\n    .await?;\n\n    let db = state\n        .database\n        .lock()\n        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;\n    ChatRepository::new(&db)\n        .list_messages(&conversation_id)?\n        .into_iter()\n        .find(|message| message.id == assistant.id)\n        .ok_or_else(|| app_error::AppError::internal("completed assistant message disappeared"))\n}''',
)

# Use the model-refined prompt/script for the actual media runtime.
replace_once(
    "src/App.tsx",
    '''      const generationKind = generationKindForMode(inferredMode);\n      if (generationKind) {\n        const artifact = await api.createGenerationArtifact(\n          conversationId,\n          assistant.id,\n          generationKind,\n          content,\n        );''',
    '''      const generationKind = generationKindForMode(inferredMode);\n      if (generationKind) {\n        const generationPrompt = assistant.content.trim() || content;\n        const artifact = await api.createGenerationArtifact(\n          conversationId,\n          assistant.id,\n          generationKind,\n          generationPrompt,\n        );''',
)
replace_once(
    "src/lib/chat.ts",
    '''    case "voice":\n      return "[Mode: Voice Creation]\\nCreate a production-quality voice generation prompt or script with voice style, pacing, emotion, pronunciation notes, format, and final narration copy. Do not claim audio was generated unless a voice generator is connected.";''',
    '''    case "voice":\n      return "[Mode: Voice Creation]\\nWrite only the final words that should be spoken aloud. Do not include headings, voice-style metadata, stage directions, markdown, or explanations. Keep punctuation natural for text-to-speech. The local voice runtime will synthesize this exact response after generation completes.";''',
)

# Remove the one-shot materializer and workflow from the production tree.
Path("scripts/apply_video_voice_runtime.py").unlink(missing_ok=True)
Path(".github/workflows/apply-video-voice-runtime.yml").unlink(missing_ok=True)
