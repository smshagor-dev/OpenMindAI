use std::path::Path;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
};

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use sha2::{Digest, Sha256};

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
    let package_dir = model_path.parent().ok_or_else(|| {
        AppError::ArtifactGenerationFailed("Kokoro package directory is unavailable".to_string())
    })?;
    // Resolve every asset passed to a third-party model loader, not just the
    // model and voices directories: a symlinked config/voice file can escape
    // the intended package without changing either parent directory.
    validate_kokoro_assets(root, package_dir, &voices_dir)?;
    // Candle reads PyTorch ZIP metadata from the primary .pth model as well
    // as from voice weights. Preflight both before entering upstream code.
    validate_kokoro_pytorch_archive(&model_path)?;
    verify_kokoro_checkpoint_digest(package_dir, &model_path)?;
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
const MAX_KOKORO_ARCHIVE_ENTRIES: usize = 4096;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const MAX_KOKORO_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const MAX_KOKORO_ZIP_EXPANSION_RATIO: u64 = 10_000;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const MAX_KOKORO_CONFIG_BYTES: u64 = 1024 * 1024;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const MAX_KOKORO_MANIFEST_BYTES: u64 = 64 * 1024;

/// Verify both the OpenMindAI root and the expected model subdirectory after
/// symlink resolution, before invoking the upstream Kokoro/Candle loader.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn validate_kokoro_assets(
    root: &PortableRootManager,
    package_dir: &Path,
    voices_dir: &Path,
) -> Result<(), AppError> {
    let config =
        canonical_file_under_root(root, &package_dir.join("config.json"), "Kokoro config.json")?;
    if !config.starts_with(package_dir) {
        return Err(AppError::ArtifactGenerationFailed(
            "Kokoro config.json resolves outside its model package".to_string(),
        ));
    }
    validate_kokoro_config(&config)?;
    let voice = canonical_file_under_root(
        root,
        &voices_dir.join(format!("{DEFAULT_VOICE}.pt")),
        "required Kokoro voice",
    )?;
    if !voice.starts_with(voices_dir) {
        return Err(AppError::ArtifactGenerationFailed(
            "Kokoro voice file resolves outside its voices directory".to_string(),
        ));
    }
    validate_kokoro_pytorch_archive(&voice)
}

/// Recheck the actual primary checkpoint against its previously recorded
/// expected hash when the package supplies one. This detects changes made
/// *after* installation, unlike comparing saved verification fields to each
/// other. A local manifest is not a signed trust anchor, and legacy installs
/// without an expected digest remain supported.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn verify_kokoro_checkpoint_digest(package_dir: &Path, model_path: &Path) -> Result<(), AppError> {
    let manifest_path = package_dir.join("model-manifest.json");
    if !manifest_path.exists() {
        return Ok(());
    }
    let canonical_package = fs::canonicalize(package_dir)?;
    let canonical = fs::canonicalize(&manifest_path)?;
    if !canonical.is_file() || !canonical.starts_with(&canonical_package) {
        return Err(AppError::ArtifactGenerationFailed(
            "Kokoro model manifest resolves outside its package".to_string(),
        ));
    }
    if fs::metadata(&canonical)?.len() > MAX_KOKORO_MANIFEST_BYTES {
        return Err(AppError::ArtifactGenerationFailed(
            "Kokoro model manifest exceeds 64 KiB".to_string(),
        ));
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(canonical)?).map_err(|error| {
            AppError::ArtifactGenerationFailed(format!("Kokoro model manifest is invalid: {error}"))
        })?;
    let Some(expected) = manifest.get("sha256").and_then(|value| value.as_str()) else {
        return Ok(());
    };
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::ArtifactGenerationFailed(
            "Kokoro model manifest has an invalid expected SHA-256".to_string(),
        ));
    }
    let filename = model_path.file_name().and_then(|name| name.to_str());
    if manifest.get("filename").and_then(|value| value.as_str()) != filename {
        return Err(AppError::ArtifactGenerationFailed(
            "Kokoro model manifest filename does not match the selected checkpoint".to_string(),
        ));
    }
    let mut file = fs::File::open(model_path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let actual = format!("{:x}", digest.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(AppError::ArtifactGenerationFailed(
            "Kokoro checkpoint changed since its expected SHA-256 was recorded; validate or re-download the model".to_string(),
        ));
    }
    Ok(())
}

/// Bound and parse model configuration before passing it to the external
/// loader. In addition to invalid JSON, excessively large configs are rejected
/// without reading them into memory.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn validate_kokoro_config(path: &Path) -> Result<(), AppError> {
    if fs::metadata(path)?.len() > MAX_KOKORO_CONFIG_BYTES {
        return Err(AppError::ArtifactGenerationFailed(
            "Kokoro config.json exceeds the 1 MiB size limit".to_string(),
        ));
    }
    let contents = fs::read(path)?;
    let parsed: serde_json::Value = serde_json::from_slice(&contents).map_err(|error| {
        AppError::ArtifactGenerationFailed(format!("Kokoro config.json is invalid: {error}"))
    })?;
    if !parsed.is_object() {
        return Err(AppError::ArtifactGenerationFailed(
            "Kokoro config.json must contain an object".to_string(),
        ));
    }
    Ok(())
}

/// Bound claimed expansion and compressed sizes before model loaders encounter
/// a checkpoint. These are metadata limits only, not proof of content integrity.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn checked_kokoro_zip_sizes(
    declared_uncompressed: u64,
    compressed: u64,
    expanded_so_far: u64,
    compressed_so_far: u64,
) -> Result<(u64, u64), AppError> {
    if declared_uncompressed > 0
        && (compressed == 0 || declared_uncompressed / compressed > MAX_KOKORO_ZIP_EXPANSION_RATIO)
    {
        return Err(AppError::ArtifactGenerationFailed(
            "Kokoro PyTorch ZIP has an implausible expansion ratio".to_string(),
        ));
    }
    let expanded = expanded_so_far
        .checked_add(declared_uncompressed)
        .ok_or_else(|| {
            AppError::ArtifactGenerationFailed("Kokoro PyTorch ZIP size overflow".to_string())
        })?;
    let stored = compressed_so_far.checked_add(compressed).ok_or_else(|| {
        AppError::ArtifactGenerationFailed("Kokoro PyTorch ZIP size overflow".to_string())
    })?;
    if expanded > MAX_KOKORO_ARCHIVE_BYTES || stored > MAX_KOKORO_ARCHIVE_BYTES {
        return Err(AppError::ArtifactGenerationFailed(
            "Kokoro PyTorch ZIP exceeds its size limit".to_string(),
        ));
    }
    Ok((expanded, stored))
}

/// PyTorch .pt and .pth files can be ZIP containers. Use the already-patched ZIP 9
/// reader to screen suspicious archive metadata before passing the file to
/// upstream Candle code still linked to ZIP 7. This does not replace or patch
/// ZIP 7 and intentionally leaves legacy non-ZIP PyTorch files to the model loader.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn validate_kokoro_pytorch_archive(path: &Path) -> Result<(), AppError> {
    let mut file = fs::File::open(path)?;
    let mut magic = [0_u8; 4];
    file.read_exact(&mut magic).map_err(|error| {
        AppError::ArtifactGenerationFailed(format!("Kokoro PyTorch archive is incomplete: {error}"))
    })?;
    if magic != *b"PK\x03\x04" {
        return Ok(());
    }
    let mut archive = zip::ZipArchive::new(file).map_err(|error| {
        AppError::ArtifactGenerationFailed(format!("Kokoro PyTorch ZIP is invalid: {error}"))
    })?;
    if archive.len() > MAX_KOKORO_ARCHIVE_ENTRIES {
        return Err(AppError::ArtifactGenerationFailed(
            "Kokoro PyTorch ZIP contains too many entries".to_string(),
        ));
    }
    let mut expanded = 0_u64;
    let mut compressed = 0_u64;
    let mut names = std::collections::HashSet::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|error| {
            AppError::ArtifactGenerationFailed(format!(
                "Kokoro PyTorch ZIP entry is invalid: {error}"
            ))
        })?;
        let enclosed = entry.enclosed_name().ok_or_else(|| {
            AppError::ArtifactGenerationFailed(
                "Kokoro PyTorch ZIP contains an unsafe path".to_string(),
            )
        })?;
        if enclosed.as_os_str().is_empty() || !names.insert(enclosed.to_path_buf()) {
            return Err(AppError::ArtifactGenerationFailed(
                "Kokoro PyTorch ZIP contains empty or duplicate paths".to_string(),
            ));
        }
        if entry.encrypted() {
            return Err(AppError::ArtifactGenerationFailed(
                "Kokoro PyTorch ZIP must not contain encrypted entries".to_string(),
            ));
        }
        if let Some(mode) = entry.unix_mode() {
            if mode & 0o7000 != 0
                || !matches!(
                    (mode & 0o170000, entry.is_dir()),
                    (0, _) | (0o040000, true) | (0o100000, false)
                )
            {
                return Err(AppError::ArtifactGenerationFailed(
                    "Kokoro PyTorch ZIP contains a link, special entry or privileged mode"
                        .to_string(),
                ));
            }
        }
        (expanded, compressed) =
            checked_kokoro_zip_sizes(entry.size(), entry.compressed_size(), expanded, compressed)?;
    }
    Ok(())
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
    fn kokoro_voice_zip_preflight_accepts_standard_archive_and_checks_count() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("af_heart.pt");
        let file = fs::File::create(&path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("voices/data.pkl", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"model-data").unwrap();
        writer.finish().unwrap();
        assert!(validate_kokoro_pytorch_archive(&path).is_ok());
        let mut file = fs::File::open(&path).unwrap();
        let mut archive = zip::ZipArchive::new(&mut file).unwrap();
        assert_eq!(archive.len(), 1);
        assert!(archive.by_name("voices/data.pkl").is_ok());
    }

    #[test]
    fn kokoro_zip_size_checks_reject_bombs_and_overflow() {
        assert!(checked_kokoro_zip_sizes(128, 128, 0, 0).is_ok());
        assert!(checked_kokoro_zip_sizes(0, 0, 0, 0).is_ok());
        assert!(checked_kokoro_zip_sizes(4, 0, 0, 0).is_err());
        assert!(checked_kokoro_zip_sizes(10_001, 1, 0, 0).is_err());
        assert!(checked_kokoro_zip_sizes(MAX_KOKORO_ARCHIVE_BYTES + 1, 1, 0, 0).is_err());
        assert!(checked_kokoro_zip_sizes(10, 10, u64::MAX, 0).is_err());
        assert!(checked_kokoro_zip_sizes(10, 10, 0, u64::MAX).is_err());
        assert!(checked_kokoro_zip_sizes(1, 1, 0, MAX_KOKORO_ARCHIVE_BYTES).is_err());
    }

    #[test]
    fn kokoro_checkpoint_digest_detects_post_install_tampering() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("kokoro");
        fs::create_dir_all(&package).unwrap();
        let model = package.join("kokoro-v1_0.pth");
        fs::write(&model, b"original-checkpoint").unwrap();
        // Legacy installs without a manifest must remain readable.
        assert!(verify_kokoro_checkpoint_digest(&package, &model).is_ok());

        let manifest = serde_json::json!({
            "filename": "kokoro-v1_0.pth",
            "sha256": format!("{:x}", Sha256::digest(b"original-checkpoint")),
        });
        fs::write(
            package.join("model-manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(verify_kokoro_checkpoint_digest(&package, &model).is_ok());
        fs::write(&model, b"tampered-checkpoint").unwrap();
        assert!(verify_kokoro_checkpoint_digest(&package, &model).is_err());
    }

    #[test]
    fn kokoro_checkpoint_digest_rejects_invalid_manifest_and_wrong_name() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("kokoro");
        fs::create_dir_all(&package).unwrap();
        let model = package.join("kokoro-v1_0.pth");
        fs::write(&model, b"checkpoint").unwrap();
        let manifest_path = package.join("model-manifest.json");
        fs::write(&manifest_path, b"broken json").unwrap();
        assert!(verify_kokoro_checkpoint_digest(&package, &model).is_err());
        let digest = format!("{:x}", Sha256::digest(b"checkpoint"));
        for (filename, sha256) in [
            ("another-file.pth", digest.as_str()),
            ("kokoro-v1_0.pth", "not-a-valid-digest"),
        ] {
            let manifest = serde_json::json!({
                "filename": filename,
                "sha256": sha256,
            });
            fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            assert!(verify_kokoro_checkpoint_digest(&package, &model).is_err());
        }
    }

    #[test]
    fn kokoro_primary_model_pth_archive_is_screened() {
        let temp = tempfile::tempdir().unwrap();
        let model_path = temp.path().join("kokoro-v1_0.pth");
        let file = fs::File::create(&model_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("kokoro/data.pkl", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"pickle-placeholder").unwrap();
        writer.finish().unwrap();
        assert!(validate_kokoro_pytorch_archive(&model_path).is_ok());
        fs::write(&model_path, b"PK\x03\x04corrupted-model").unwrap();
        assert!(validate_kokoro_pytorch_archive(&model_path).is_err());
    }

    #[test]
    fn kokoro_config_must_be_bounded_valid_json_object() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        fs::write(&path, b"{\"model\":\"kokoro\"}").unwrap();
        assert!(validate_kokoro_config(&path).is_ok());
        fs::write(&path, b"not json").unwrap();
        assert!(validate_kokoro_config(&path).is_err());
        fs::write(&path, b"[]").unwrap();
        assert!(validate_kokoro_config(&path).is_err());
        fs::write(&path, vec![b'x'; MAX_KOKORO_CONFIG_BYTES as usize + 1]).unwrap();
        assert!(validate_kokoro_config(&path).is_err());
    }

    #[test]
    fn kokoro_voice_zip_preflight_rejects_invalid_zip() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("af_heart.pt");
        fs::write(&path, b"PK\x03\x04not-a-real-zip").unwrap();
        assert!(validate_kokoro_pytorch_archive(&path).is_err());
    }

    #[test]
    fn kokoro_assets_require_files_inside_the_expected_package() {
        let temp = tempfile::tempdir().unwrap();
        let root_dir = temp.path().join("OpenMindAI");
        let package = root_dir.join("models/kokoro");
        let voices = package.join("voices");
        fs::create_dir_all(&voices).unwrap();
        fs::write(package.join("config.json"), b"{}").unwrap();
        fs::write(voices.join("af_heart.pt"), b"legacy-model-data").unwrap();
        let root = PortableRootManager::from_root(root_dir);
        let package = fs::canonicalize(package).unwrap();
        let voices = fs::canonicalize(voices).unwrap();
        assert!(validate_kokoro_assets(&root, &package, &voices).is_ok());
        fs::remove_file(voices.join("af_heart.pt")).unwrap();
        assert!(validate_kokoro_assets(&root, &package, &voices).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn kokoro_assets_reject_symlinked_files_outside_their_directories() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root_dir = temp.path().join("OpenMindAI");
        let package = root_dir.join("models/kokoro");
        let voices = package.join("voices");
        let elsewhere = root_dir.join("unrelated");
        fs::create_dir_all(&voices).unwrap();
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(elsewhere.join("config.json"), b"{}").unwrap();
        fs::write(elsewhere.join("af_heart.pt"), b"external-voice").unwrap();
        fs::write(package.join("config.json"), b"{}").unwrap();
        symlink(elsewhere.join("af_heart.pt"), voices.join("af_heart.pt")).unwrap();
        let root = PortableRootManager::from_root(root_dir);
        let package = fs::canonicalize(package).unwrap();
        let voices = fs::canonicalize(voices).unwrap();
        assert!(validate_kokoro_assets(&root, &package, &voices).is_err());
        fs::remove_file(voices.join("af_heart.pt")).unwrap();
        fs::write(voices.join("af_heart.pt"), b"local-voice").unwrap();
        fs::remove_file(package.join("config.json")).unwrap();
        symlink(elsewhere.join("config.json"), package.join("config.json")).unwrap();
        assert!(validate_kokoro_assets(&root, &package, &voices).is_err());
    }

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
