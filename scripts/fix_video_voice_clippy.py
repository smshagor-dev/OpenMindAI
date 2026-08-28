from pathlib import Path
import json


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    if old not in text:
        raise SystemExit(f"anchor not found in {path}: {old[:160]!r}")
    file.write_text(text.replace(old, new, 1), encoding="utf-8")


# Keep video generation strict-Clippy clean by grouping the runtime request.
replace_once(
    "src-tauri/src/diffusion_runtime.rs",
    '''pub async fn generate_video(
    root: &PortableRootManager,
    client: &Client,
    hardware: &HardwareProfile,
    diffusion_model_path: &Path,
    vae_path: &Path,
    text_encoder_path: &Path,
    prompt: &str,
    output_path: &Path,
) -> Result<(), AppError> {
    let prompt = prompt.trim();''',
    '''pub(crate) struct VideoGenerationRequest<'a> {
    pub diffusion_model_path: &'a Path,
    pub vae_path: &'a Path,
    pub text_encoder_path: &'a Path,
    pub prompt: &'a str,
    pub output_path: &'a Path,
}

pub async fn generate_video(
    root: &PortableRootManager,
    client: &Client,
    hardware: &HardwareProfile,
    request: VideoGenerationRequest<'_>,
) -> Result<(), AppError> {
    let VideoGenerationRequest {
        diffusion_model_path,
        vae_path,
        text_encoder_path,
        prompt,
        output_path,
    } = request;
    let prompt = prompt.trim();''',
)

replace_once(
    "src-tauri/src/lib.rs",
    '''            diffusion_runtime::generate_video(
                &state.root,
                &state.http,
                &hardware,
                &model_path,
                &vae_path,
                &text_encoder_path,
                prompt,
                path,
            )
            .await''',
    '''            diffusion_runtime::generate_video(
                &state.root,
                &state.http,
                &hardware,
                diffusion_runtime::VideoGenerationRequest {
                    diffusion_model_path: &model_path,
                    vae_path: &vae_path,
                    text_encoder_path: &text_encoder_path,
                    prompt,
                    output_path: path,
                },
            )
            .await''',
)

# kokoro-en currently enables ort's downloaded prebuilt ONNX Runtime through a
# non-optional dependency. Those Linux binaries require newer glibc/libstdc++
# symbols than Ubuntu 22.04, which breaks our supported Linux CI at link time.
# Use the native Candle Kokoro backend instead: no Python, subprocess, or ORT.
replace_once(
    "src-tauri/Cargo.toml",
    'kokoro-en = "0.1.5"\n',
    'any-tts = { version = "0.1.3", default-features = false, features = ["kokoro"] }\n',
)

catalog_path = Path("src-tauri/model-catalog.json")
catalog = json.loads(catalog_path.read_text(encoding="utf-8"))
speak = next(model for model in catalog["models"] if model["id"] == "kokoro-82m-onnx")
speak.update(
    {
        "version": "1.0",
        "family": "kokoro",
        "runtime": "any-tts-candle",
        "repo": "hexgrad/Kokoro-82M",
        "quantization": "FP32",
        "sizeBytes": 328000000,
        "minRamBytes": 4294967296,
        "minVramBytes": None,
        "description": "Native local Kokoro speech synthesis through Rust/Candle with in-process phonemization and verified voice weights.",
        "download": {
            "strategy": "singleFile",
            "filenamePattern": "*kokoro-v1_0.pth",
            "destinationDir": "models/audio/kokoro",
            "format": "pth",
            "dependencies": [
                {
                    "role": "config",
                    "filenamePattern": "*config.json",
                    "format": "json",
                    "required": True,
                },
                {
                    "role": "voice",
                    "filenamePattern": "*af_heart.pt",
                    "format": "pt",
                    "required": True,
                },
            ],
        },
    }
)
catalog_path.write_text(json.dumps(catalog, indent=2) + "\n", encoding="utf-8")

replace_once(
    "src-tauri/src/model_catalog.rs",
    '        assert_eq!(speak.runtime, "kokoro-en");',
    '        assert_eq!(speak.runtime, "any-tts-candle");',
)

# Native Candle TTS stays on a blocking worker so model loading/inference does
# not stall Tauri's async command runtime. The package root remains under the
# portable OpenMindAI root and contains config.json, the .pth model, and voice.
Path("src-tauri/src/voice_runtime.rs").write_text(
    r'''use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use any_tts::{
    models::kokoro::KokoroModel,
    traits::TtsModel,
    ModelType, SynthesisRequest, TtsConfig,
};

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
    if !voices_dir.join(format!("{DEFAULT_VOICE}.pt")).is_file() {
        return Err(AppError::ArtifactGenerationFailed(format!(
            "required Kokoro voice {DEFAULT_VOICE} is missing"
        )));
    }
    let package_dir = model_path.parent().ok_or_else(|| {
        AppError::ArtifactGenerationFailed("Kokoro package directory is unavailable".to_string())
    })?;
    if !package_dir.join("config.json").is_file() {
        return Err(AppError::ArtifactGenerationFailed(
            "required Kokoro config.json is missing; validate or re-download the model package"
                .to_string(),
        ));
    }
    let package_dir = package_dir.to_path_buf();

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

    tracing::info!(voice = DEFAULT_VOICE, "loading local Rust/Candle Kokoro voice runtime");
    let narration = text.to_string();
    let audio = tokio::task::spawn_blocking(move || -> Result<any_tts::AudioSamples, String> {
        let config = TtsConfig::new(ModelType::Kokoro).with_model_path(&package_dir);
        let model = KokoroModel::load(config)
            .map_err(|error| format!("could not load Kokoro TTS: {error}"))?;
        let request = SynthesisRequest::new(narration)
            .with_language("en")
            .with_voice(DEFAULT_VOICE);
        model
            .synthesize(&request)
            .map_err(|error| format!("Kokoro speech synthesis failed: {error}"))
    })
    .await
    .map_err(|error| {
        AppError::ArtifactGenerationFailed(format!("Kokoro synthesis worker failed: {error}"))
    })?
    .map_err(AppError::ArtifactGenerationFailed)?;

    if audio.is_empty() {
        return Err(AppError::ArtifactGenerationFailed(
            "Kokoro returned no audio samples".to_string(),
        ));
    }
    if audio.sample_rate != SAMPLE_RATE || audio.channels != 1 {
        return Err(AppError::ArtifactGenerationFailed(format!(
            "Kokoro returned unsupported audio format: {} Hz / {} channel(s)",
            audio.sample_rate, audio.channels
        )));
    }

    write_pcm16_wav(output_path, &audio.samples)?;
    validate_wav(output_path)?;
    tracing::info!(
        path = %output_path.display(),
        samples = audio.samples.len(),
        "local Rust/Candle Kokoro WAV generated"
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

# Retry generated media through the media runtime, not the generic text/file
# artifact path.
replace_once(
    "src/App.tsx",
    '''  const retryArtifact = useCallback(
    (artifact: Artifact) => {
      const source = messages.find((message) => message.id === artifact.messageId);
      if (!source || !artifact.messageId) {
        showError("The original message for this file is no longer available.");
        return;
      }
      void createArtifact(artifact.messageId, artifact.kind, source.content, artifact.name);
    },
    [createArtifact, messages, showError],
  );''',
    '''  const retryArtifact = useCallback(
    (artifact: Artifact) => {
      const source = messages.find((message) => message.id === artifact.messageId);
      if (!source || !artifact.messageId || !activeId) {
        showError("The original message for this file is no longer available.");
        return;
      }
      if (artifact.kind === "image" || artifact.kind === "video" || artifact.kind === "audio") {
        const generationKind = artifact.kind === "audio" ? "voice" : artifact.kind;
        void api
          .createGenerationArtifact(activeId, artifact.messageId, generationKind, source.content)
          .then((next) => {
            setArtifacts((items) => upsertArtifactInList(items, next));
            if (preferences?.openArtifactsAfterGeneration && next.status === "ready") {
              void api.openArtifact(next.id).catch(showError);
            }
          })
          .catch(showError);
        return;
      }
      void createArtifact(artifact.messageId, artifact.kind, source.content, artifact.name);
    },
    [activeId, createArtifact, messages, preferences?.openArtifactsAfterGeneration, showError],
  );''',
)

replace_once(
    "src/lib/chat.ts",
    '''    case "video":
      return "[Mode: Video Creation]\\nCreate a production-quality video generation prompt with subject, scene progression, camera motion, timing, lighting, style, negative prompt, and recommended duration/aspect ratio. Do not claim a video file was generated unless a video generator is connected.";''',
    '''    case "video":
      return "[Mode: Video Creation]\\nWrite only the final positive visual prompt for the local video renderer. Describe subject, environment, action, scene progression, camera motion, lighting, composition, and visual style in natural prose. Do not add headings, markdown, negative prompts, duration, aspect-ratio recommendations, explanations, or claims that rendering already succeeded; the local runtime controls those settings separately.";''',
)

Path("scripts/fix_video_voice_clippy.py").unlink(missing_ok=True)
