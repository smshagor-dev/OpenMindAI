use std::path::Path;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
};

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use any_tts::{
    models::kokoro::KokoroModel, traits::TtsModel, ModelType, SynthesisRequest, TtsConfig,
};

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::model_download::ensure_contained;
use crate::{app_error::AppError, portable_root::PortableRootManager};

#[cfg(not(any(target_os = "android", target_os = "ios")))]
const SAMPLE_RATE: u32 = 24_000;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const DEFAULT_VOICE: &str = "af_heart";
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const MAX_TTS_CHARS: usize = 12_000;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const WAV_HEADER_BYTES: u64 = 44;

#[cfg(any(target_os = "android", target_os = "ios"))]
pub async fn generate_voice(
    root: &PortableRootManager,
    model_path: &Path,
    voices_dir: &Path,
    text: &str,
    output_path: &Path,
) -> Result<(), AppError> {
    let _ = (root, model_path, voices_dir, text, output_path);
    Err(AppError::ArtifactGenerationFailed(
        "Local OpenMindAI Speak is not enabled in the Android/iOS app shell yet. Mobile voice synthesis is planned for the mobile inference phase."
            .to_string(),
    ))
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
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

    tracing::info!(
        voice = DEFAULT_VOICE,
        "loading local Rust/Candle Kokoro voice runtime"
    );
    let narration = text.to_string();
    let audio = tokio::task::spawn_blocking(move || -> Result<any_tts::AudioSamples, String> {
        let config = TtsConfig::new(ModelType::Kokoro)
            .with_model_path(package_dir.to_string_lossy().into_owned());
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

#[cfg(not(any(target_os = "android", target_os = "ios")))]
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

#[cfg(not(any(target_os = "android", target_os = "ios")))]
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

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn write_pcm16_wav(path: &Path, samples: &[f32]) -> Result<(), AppError> {
    let data_bytes = samples
        .len()
        .checked_mul(2)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            AppError::ArtifactGenerationFailed("voice output is too large".to_string())
        })?;
    let riff_size = 36_u32.checked_add(data_bytes).ok_or_else(|| {
        AppError::ArtifactGenerationFailed("voice output is too large".to_string())
    })?;

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

#[cfg(not(any(target_os = "android", target_os = "ios")))]
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

#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
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
        assert_eq!(
            fs::metadata(path).unwrap().len(),
            44 + samples.len() as u64 * 2
        );
    }

    #[test]
    fn rejects_non_wav_output() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("voice.wav");
        fs::write(&path, vec![0_u8; 128]).unwrap();
        assert!(validate_wav(&path).is_err());
    }
}
