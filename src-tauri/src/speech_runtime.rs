use std::{io::Cursor, path::Path, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use hound::{SampleFormat, WavReader};
use serde::Serialize;
use whisper_rs::{
    get_lang_str, FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
};

use crate::{app_error::AppError, portable_root::PortableRootManager};

const TARGET_SAMPLE_RATE: u32 = 16_000;
const MAX_AUDIO_BYTES: usize = 64 * 1024 * 1024;
const MAX_AUDIO_DURATION: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionResult {
    pub text: String,
    pub language: Option<String>,
    pub duration_seconds: f64,
}

pub async fn transcribe_data_url(
    root: &PortableRootManager,
    model_path: &Path,
    data_url: &str,
    source_name: &str,
) -> Result<TranscriptionResult, AppError> {
    let model_path = canonical_model(root, model_path)?;
    let wav_bytes = decode_wav_data_url(data_url)?;
    let (samples, duration_seconds) = decode_wav_to_16khz_mono(&wav_bytes)?;
    if samples.is_empty() {
        return Err(AppError::InferenceFailed(
            "The audio contains no decodable samples.".to_string(),
        ));
    }
    if duration_seconds > MAX_AUDIO_DURATION.as_secs_f64() {
        return Err(AppError::InferenceFailed(format!(
            "Audio is too long for local transcription. Maximum supported duration is {} minutes.",
            MAX_AUDIO_DURATION.as_secs() / 60
        )));
    }

    let source_name = source_name.to_string();
    tokio::task::spawn_blocking(move || {
        tracing::info!(
            source = %source_name,
            duration_seconds,
            model = %model_path.display(),
            "starting local Whisper transcription"
        );

        let context = WhisperContext::new_with_params(
            model_path.to_string_lossy().as_ref(),
            WhisperContextParameters::default(),
        )
        .map_err(|error| {
            AppError::InferenceFailed(format!("Could not load OpenMindAI Hear: {error}"))
        })?;
        let mut state = context.create_state().map_err(|error| {
            AppError::InferenceFailed(format!("Could not create Whisper state: {error}"))
        })?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(
            std::thread::available_parallelism()
                .map(|value| value.get().min(8) as i32)
                .unwrap_or(4),
        );
        params.set_translate(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_no_context(true);

        state.full(params, &samples).map_err(|error| {
            AppError::InferenceFailed(format!("OpenMindAI Hear transcription failed: {error}"))
        })?;

        let text = state
            .as_iter()
            .map(|segment| segment.to_string())
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string();
        if text.is_empty() {
            return Err(AppError::InferenceFailed(
                "OpenMindAI Hear did not detect speech in this audio.".to_string(),
            ));
        }
        let language = get_lang_str(state.full_lang_id_from_state()).map(ToString::to_string);
        tracing::info!(
            source = %source_name,
            language = ?language,
            transcript_chars = text.chars().count(),
            "local Whisper transcription completed"
        );
        Ok(TranscriptionResult {
            text,
            language,
            duration_seconds,
        })
    })
    .await
    .map_err(|error| AppError::InferenceFailed(format!("Whisper worker failed: {error}")))?
}

fn canonical_model(root: &PortableRootManager, model_path: &Path) -> Result<std::path::PathBuf, AppError> {
    let canonical_root = std::fs::canonicalize(root.root())?;
    let canonical = std::fs::canonicalize(model_path).map_err(|error| {
        AppError::InferenceFailed(format!("OpenMindAI Hear model is unavailable: {error}"))
    })?;
    if !canonical.is_file() || !canonical.starts_with(canonical_root) {
        return Err(AppError::InferenceFailed(
            "OpenMindAI Hear model path is invalid or outside OpenMindAI Root.".to_string(),
        ));
    }
    Ok(canonical)
}

fn decode_wav_data_url(data_url: &str) -> Result<Vec<u8>, AppError> {
    let (metadata, encoded) = data_url.split_once(',').ok_or_else(|| {
        AppError::InferenceFailed("Audio payload is not a valid data URL.".to_string())
    })?;
    if !metadata.starts_with("data:audio/wav") || !metadata.ends_with(";base64") {
        return Err(AppError::InferenceFailed(
            "Local transcription expects a PCM WAV payload.".to_string(),
        ));
    }
    let decoded = STANDARD.decode(encoded).map_err(|error| {
        AppError::InferenceFailed(format!("Audio payload could not be decoded: {error}"))
    })?;
    if decoded.is_empty() || decoded.len() > MAX_AUDIO_BYTES {
        return Err(AppError::InferenceFailed(format!(
            "Audio payload must be between 1 byte and {} MB.",
            MAX_AUDIO_BYTES / (1024 * 1024)
        )));
    }
    Ok(decoded)
}

fn decode_wav_to_16khz_mono(bytes: &[u8]) -> Result<(Vec<f32>, f64), AppError> {
    let mut reader = WavReader::new(Cursor::new(bytes)).map_err(|error| {
        AppError::InferenceFailed(format!("Could not decode WAV audio: {error}"))
    })?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.sample_rate == 0 {
        return Err(AppError::InferenceFailed(
            "WAV audio has an invalid channel count or sample rate.".to_string(),
        ));
    }

    let raw = match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Float, 32) => reader
            .samples::<f32>()
            .map(|sample| sample.map(|value| value.clamp(-1.0, 1.0)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| AppError::InferenceFailed(format!("Could not read WAV samples: {error}")))?,
        (SampleFormat::Int, bits) if bits <= 16 => {
            let scale = i16::MAX as f32;
            reader
                .samples::<i16>()
                .map(|sample| sample.map(|value| value as f32 / scale))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| AppError::InferenceFailed(format!("Could not read WAV samples: {error}")))?
        }
        (SampleFormat::Int, bits) if bits <= 32 => {
            let scale = ((1_i64 << (bits.saturating_sub(1) as u32)) - 1).max(1) as f32;
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|value| (value as f32 / scale).clamp(-1.0, 1.0)))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| AppError::InferenceFailed(format!("Could not read WAV samples: {error}")))?
        }
        _ => {
            return Err(AppError::InferenceFailed(format!(
                "Unsupported WAV format: {:?} / {} bit.",
                spec.sample_format, spec.bits_per_sample
            )))
        }
    };

    let channels = spec.channels as usize;
    let mono = raw
        .chunks(channels)
        .map(|frame| frame.iter().copied().sum::<f32>() / frame.len().max(1) as f32)
        .collect::<Vec<_>>();
    let duration_seconds = mono.len() as f64 / spec.sample_rate as f64;
    if spec.sample_rate == TARGET_SAMPLE_RATE {
        return Ok((mono, duration_seconds));
    }

    let target_len = (duration_seconds * TARGET_SAMPLE_RATE as f64).round() as usize;
    if target_len == 0 || mono.is_empty() {
        return Ok((Vec::new(), duration_seconds));
    }
    let ratio = spec.sample_rate as f64 / TARGET_SAMPLE_RATE as f64;
    let mut resampled = Vec::with_capacity(target_len);
    for target_index in 0..target_len {
        let position = target_index as f64 * ratio;
        let left = position.floor() as usize;
        let right = (left + 1).min(mono.len() - 1);
        let fraction = (position - left as f64) as f32;
        let sample = mono[left] * (1.0 - fraction) + mono[right] * fraction;
        resampled.push(sample.clamp(-1.0, 1.0));
    }
    Ok((resampled, duration_seconds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::{WavSpec, WavWriter};

    fn test_wav(sample_rate: u32, channels: u16) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let spec = WavSpec {
                channels,
                sample_rate,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            };
            let mut writer = WavWriter::new(&mut cursor, spec).unwrap();
            for index in 0..sample_rate {
                let sample = ((index as f32 / 30.0).sin() * 10_000.0) as i16;
                for _ in 0..channels {
                    writer.write_sample(sample).unwrap();
                }
            }
            writer.finalize().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn decodes_and_resamples_wav() {
        let bytes = test_wav(48_000, 2);
        let (samples, duration) = decode_wav_to_16khz_mono(&bytes).unwrap();
        assert!((duration - 1.0).abs() < 0.01);
        assert!((samples.len() as isize - 16_000).abs() < 4);
    }

    #[test]
    fn validates_audio_data_url() {
        let bytes = test_wav(16_000, 1);
        let encoded = format!("data:audio/wav;base64,{}", STANDARD.encode(bytes));
        assert!(!decode_wav_data_url(&encoded).unwrap().is_empty());
        assert!(decode_wav_data_url("data:text/plain;base64,SGVsbG8=").is_err());
    }
}
