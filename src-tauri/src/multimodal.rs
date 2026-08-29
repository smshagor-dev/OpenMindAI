use std::{fs, io::Cursor, path::Path};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use hound::{SampleFormat, WavSpec, WavWriter};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, State};

use crate::{
    app_error::AppError,
    artifacts::{Artifact, ArtifactManager, ArtifactRepository},
    chat::ChatRepository,
    model_catalog,
    speech_runtime::{self, TranscriptionResult},
    AppState,
};

const MAX_INLINE_MEDIA_BYTES: u64 = 128 * 1024 * 1024;
const SOUNDSCAPE_SAMPLE_RATE: u32 = 44_100;
const DEFAULT_SOUNDSCAPE_SECONDS: f32 = 8.0;
const MAX_SOUNDSCAPE_SECONDS: f32 = 30.0;

#[tauri::command]
pub(crate) async fn transcribe_audio(
    audio_data_url: String,
    source_name: String,
    state: State<'_, AppState>,
) -> Result<TranscriptionResult, AppError> {
    let model = crate::installed_catalog_entry_by_id(&state, "whisper-large-v3-turbo-q5")?
        .ok_or_else(|| {
            AppError::InferenceFailed(
                "OpenMindAI Hear is not installed. Open Settings > Models and download OpenMindAI Hear first."
                    .to_string(),
            )
        })?;
    let relative = model.installed_path.as_deref().ok_or_else(|| {
        AppError::InferenceFailed("OpenMindAI Hear model path is unavailable.".to_string())
    })?;
    let model_path = state.root.resolve_relative(relative)?;
    speech_runtime::transcribe_data_url(
        &state.root,
        &model_path,
        &audio_data_url,
        source_name.trim(),
    )
    .await
}

#[tauri::command]
pub(crate) fn artifact_media_data_url(
    artifact_id: String,
    state: State<AppState>,
) -> Result<String, AppError> {
    let artifact = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        ArtifactRepository::new(&db).find(&artifact_id)?
    };
    if artifact.status != "ready" || !matches!(artifact.kind.as_str(), "image" | "audio") {
        return Err(AppError::internal(
            "only ready image and audio artifacts can be previewed inline",
        ));
    }
    let path = state.root.resolve_relative(&artifact.path)?;
    let metadata = fs::metadata(&path)?;
    if !metadata.is_file() || metadata.len() > MAX_INLINE_MEDIA_BYTES {
        return Err(AppError::internal(
            "artifact is unavailable or too large for inline preview",
        ));
    }
    let bytes = fs::read(path)?;
    Ok(format!(
        "data:{};base64,{}",
        artifact.mime_type,
        STANDARD.encode(bytes)
    ))
}

#[tauri::command]
pub(crate) async fn create_soundscape_artifact(
    app: AppHandle,
    conversation_id: String,
    message_id: Option<String>,
    prompt: String,
    state: State<'_, AppState>,
) -> Result<Artifact, AppError> {
    let prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(AppError::ArtifactGenerationFailed(
            "music or sound-effect prompt cannot be empty".to_string(),
        ));
    }
    state.root.validate_root()?;

    // Stable Audio Open currently requires gated Hugging Face access. Keep the
    // catalog model discoverable as an optional quality engine, but never make
    // basic local Music/SFX generation fail simply because a gated model token
    // is unavailable. The built-in deterministic synthesizer is the reliable
    // offline fallback and produces a standard PCM WAV on every supported OS.
    let stable_audio_installed = crate::installed_catalog_entry_by_id(&state, "stable-audio-open")?
        .is_some();
    tracing::info!(stable_audio_installed, "preparing OpenMindAI Soundscape generation");

    let (artifact, path) = {
        let db = state
            .database
            .lock()
            .map_err(|_| AppError::internal("database lock poisoned"))?;
        let fallback_title = ChatRepository::new(&db)
            .find_conversation(&conversation_id)
            .map(|conversation| conversation.title)
            .unwrap_or_else(|_| "soundscape".to_string());
        let manager = ArtifactManager::new(&state.root);
        let (path, filename, relative) =
            manager.resolve_destination("audio", Some("soundscape.wav"), &fallback_title)?;
        let artifacts = ArtifactRepository::new(&db);
        let artifact = artifacts.create(
            &conversation_id,
            message_id.as_deref(),
            &filename,
            &relative,
            "audio/wav",
            "audio",
            "generating",
        )?;
        (artifact, path)
    };

    app.emit("artifact:started", &artifact)
        .map_err(|error| AppError::internal(error.to_string()))?;

    let generation_prompt = prompt.clone();
    let generation_path = path.clone();
    let generation_result = tokio::task::spawn_blocking(move || {
        generate_soundscape_wav(&generation_prompt, &generation_path)
    })
    .await
    .map_err(|error| AppError::ArtifactGenerationFailed(format!("sound worker failed: {error}")))?;

    let db = state
        .database
        .lock()
        .map_err(|_| AppError::internal("database lock poisoned"))?;
    let artifacts = ArtifactRepository::new(&db);
    let updated = match generation_result {
        Ok(()) => {
            let size = fs::metadata(&path).map(|meta| meta.len() as i64).unwrap_or(0);
            artifacts.set_ready(&artifact.id, size, None)?;
            artifacts.find(&artifact.id)?
        }
        Err(error) => {
            artifacts.set_failed(&artifact.id, &error.to_string())?;
            artifacts.find(&artifact.id)?
        }
    };
    drop(db);
    app.emit("artifact:done", &updated)
        .map_err(|error| AppError::internal(error.to_string()))?;
    Ok(updated)
}

fn generate_soundscape_wav(prompt: &str, output: &Path) -> Result<(), AppError> {
    let duration = requested_duration(prompt).clamp(2.0, MAX_SOUNDSCAPE_SECONDS);
    let frames = (duration * SOUNDSCAPE_SAMPLE_RATE as f32) as usize;
    let mut writer = WavWriter::create(
        output,
        WavSpec {
            channels: 2,
            sample_rate: SOUNDSCAPE_SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        },
    )
    .map_err(|error| AppError::ArtifactGenerationFailed(error.to_string()))?;

    let normalized = prompt.to_ascii_lowercase();
    let mut seed_bytes = [0_u8; 8];
    seed_bytes.copy_from_slice(&Sha256::digest(prompt.as_bytes())[..8]);
    let mut rng = XorShift64::new(u64::from_le_bytes(seed_bytes));
    let bpm = requested_bpm(&normalized).unwrap_or(100.0).clamp(50.0, 190.0);
    let beat_seconds = 60.0 / bpm;

    for index in 0..frames {
        let t = index as f32 / SOUNDSCAPE_SAMPLE_RATE as f32;
        let noise = rng.next_f32() * 2.0 - 1.0;
        let mut sample = if normalized.contains("rain") {
            rain_sample(t, noise, &mut rng)
        } else if normalized.contains("ocean") || normalized.contains("wave") || normalized.contains("sea") {
            ocean_sample(t, noise)
        } else if normalized.contains("wind") {
            wind_sample(t, noise)
        } else if normalized.contains("beep")
            || normalized.contains("notification")
            || normalized.contains("alert")
            || normalized.contains("ui sound")
        {
            notification_sample(t)
        } else {
            music_sample(t, beat_seconds, &normalized, noise)
        };

        let fade = edge_fade(t, duration);
        sample = (sample * fade).clamp(-0.95, 0.95);
        let stereo_motion = (t * 0.37).sin() * 0.08;
        let left = (sample * (1.0 - stereo_motion)).clamp(-1.0, 1.0);
        let right = (sample * (1.0 + stereo_motion)).clamp(-1.0, 1.0);
        writer
            .write_sample((left * i16::MAX as f32) as i16)
            .and_then(|_| writer.write_sample((right * i16::MAX as f32) as i16))
            .map_err(|error| AppError::ArtifactGenerationFailed(error.to_string()))?;
    }
    writer
        .finalize()
        .map_err(|error| AppError::ArtifactGenerationFailed(error.to_string()))?;
    validate_wav(output)
}

fn rain_sample(t: f32, noise: f32, rng: &mut XorShift64) -> f32 {
    let bed = noise * 0.16;
    let drops = if rng.next_f32() > 0.997 {
        (2.0 * std::f32::consts::PI * (1200.0 + rng.next_f32() * 2600.0) * t).sin() * 0.22
    } else {
        0.0
    };
    bed + drops
}

fn ocean_sample(t: f32, noise: f32) -> f32 {
    let swell = ((t * 0.42).sin() * 0.5 + 0.5).powf(1.8);
    noise * (0.05 + swell * 0.18) + (t * 2.1).sin() * 0.025
}

fn wind_sample(t: f32, noise: f32) -> f32 {
    let gust = ((t * 0.31).sin() * 0.5 + 0.5) * ((t * 0.071).sin() * 0.5 + 0.5);
    noise * (0.04 + gust * 0.20)
}

fn notification_sample(t: f32) -> f32 {
    let note = if t < 0.22 {
        880.0
    } else if t < 0.48 {
        1174.66
    } else {
        1567.98
    };
    let local = t % 0.75;
    let envelope = if local < 0.5 { (1.0 - local / 0.5).powf(2.0) } else { 0.0 };
    (2.0 * std::f32::consts::PI * note * t).sin() * envelope * 0.32
}

fn music_sample(t: f32, beat_seconds: f32, prompt: &str, noise: f32) -> f32 {
    let minor = prompt.contains("dark") || prompt.contains("lofi") || prompt.contains("cinematic");
    let root = if prompt.contains("bright") { 261.63 } else { 220.0 };
    let thirds = if minor { 1.1892 } else { 1.2599 };
    let chord = [root, root * thirds, root * 1.4983];
    let bar = ((t / beat_seconds) as usize / 4) % 4;
    let transpose = [1.0, 1.3348, 1.4983, 1.1892][bar];
    let pad = chord
        .iter()
        .enumerate()
        .map(|(index, freq)| {
            let wobble = 1.0 + (t * (0.11 + index as f32 * 0.03)).sin() * 0.002;
            (2.0 * std::f32::consts::PI * freq * transpose * wobble * t).sin()
        })
        .sum::<f32>()
        / chord.len() as f32
        * 0.17;

    let beat_phase = (t / beat_seconds).fract();
    let kick = if beat_phase < 0.16 {
        let env = (1.0 - beat_phase / 0.16).powf(2.5);
        let freq = 52.0 + 55.0 * env;
        (2.0 * std::f32::consts::PI * freq * t).sin() * env * 0.30
    } else {
        0.0
    };
    let half_phase = (t / (beat_seconds * 2.0)).fract();
    let snare = if (0.48..0.62).contains(&half_phase) {
        noise * (1.0 - (half_phase - 0.48) / 0.14) * 0.12
    } else {
        0.0
    };
    let eighth = (t / (beat_seconds / 2.0)).fract();
    let hat = if eighth < 0.06 {
        noise * (1.0 - eighth / 0.06) * 0.045
    } else {
        0.0
    };
    pad + kick + snare + hat
}

fn requested_duration(prompt: &str) -> f32 {
    let tokens = prompt.split_whitespace().collect::<Vec<_>>();
    for window in tokens.windows(2) {
        let value = window[0]
            .trim_matches(|character: char| !character.is_ascii_digit() && character != '.')
            .parse::<f32>();
        if let Ok(value) = value {
            let unit = window[1].to_ascii_lowercase();
            if unit.starts_with("sec") || unit == "s" || unit.starts_with("second") {
                return value;
            }
        }
    }
    DEFAULT_SOUNDSCAPE_SECONDS
}

fn requested_bpm(prompt: &str) -> Option<f32> {
    let tokens = prompt.split_whitespace().collect::<Vec<_>>();
    for (index, token) in tokens.iter().enumerate() {
        if token.eq_ignore_ascii_case("bpm") && index > 0 {
            if let Ok(value) = tokens[index - 1].trim_matches(|c: char| !c.is_ascii_digit()).parse() {
                return Some(value);
            }
        }
    }
    None
}

fn edge_fade(t: f32, duration: f32) -> f32 {
    let fade_in = (t / 0.08).clamp(0.0, 1.0);
    let fade_out = ((duration - t) / 0.20).clamp(0.0, 1.0);
    fade_in * fade_out
}

fn validate_wav(path: &Path) -> Result<(), AppError> {
    let bytes = fs::read(path)?;
    if bytes.len() <= 44 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(AppError::ArtifactGenerationFailed(
            "Soundscape runtime did not produce a valid WAV file.".to_string(),
        ));
    }
    Ok(())
}

struct XorShift64(u64);

impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0x9e3779b97f4a7c15 } else { seed })
    }

    fn next_f32(&mut self) -> f32 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        (value as f64 / u64::MAX as f64) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soundscape_generator_creates_valid_wav() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sound.wav");
        generate_soundscape_wav("8 second ambient lofi at 90 BPM", &path).unwrap();
        let bytes = fs::read(path).unwrap();
        assert!(bytes.len() > 44);
        assert_eq!(&bytes[..4], b"RIFF");
    }

    #[test]
    fn parses_duration_and_bpm() {
        assert_eq!(requested_duration("make 6 seconds of rain"), 6.0);
        assert_eq!(requested_bpm("ambient 128 BPM loop"), Some(128.0));
    }
}
