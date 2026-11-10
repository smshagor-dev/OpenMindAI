use crate::{
    app_error::AppError,
    model_catalog::{wildcard_match, ModelCatalogEntry},
    model_registry::{ModelLifecycleState, ModelRecord},
};
use serde_json::Value;
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
pub(super) struct LensPaths {
    pub checkpoint: PathBuf,
    pub projector: PathBuf,
}

fn invalid() -> AppError {
    AppError::ModelInvalid("OpenMindAI Lens package is incomplete or mismatched. Validate or re-download Lens from Settings > Models.".into())
}
fn contained_file(root: &Path, path: &Path) -> Result<PathBuf, AppError> {
    let canonical = fs::canonicalize(path).map_err(|_| invalid())?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(invalid());
    }
    Ok(canonical)
}
fn manifest(root: &Path, directory: &Path, filename: &str) -> Result<Value, AppError> {
    let path = contained_file(root, &directory.join(filename))?;
    if !path.starts_with(directory) {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(invalid());
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}
fn package_file(root: &Path, directory: &Path, filename: &str) -> Result<PathBuf, AppError> {
    let relative = Path::new(filename);
    if filename.is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(invalid());
    }
    let file = contained_file(root, &directory.join(relative))?;
    if !file.starts_with(directory) {
        return Err(invalid());
    }
    Ok(file)
}

/// Resolve the catalog package's declared checkpoint and projector, never a
/// same-family model or whichever mmproj happens to sort first in a directory.
pub(super) fn lens_paths(root: &Path, entry: &ModelCatalogEntry) -> Result<LensPaths, AppError> {
    let root = fs::canonicalize(root)?;
    let download = entry.download.as_ref().ok_or_else(invalid)?;
    let directory =
        fs::canonicalize(root.join(&download.destination_dir)).map_err(|_| invalid())?;
    if !directory.starts_with(&root) {
        return Err(invalid());
    }
    let primary = manifest(&root, &directory, "model-manifest.json")?;
    let filename = primary["filename"].as_str().ok_or_else(invalid)?;
    if primary["repo"].as_str() != Some(entry.repo.as_str())
        || primary["quantization"].as_str() != Some(entry.quantization.as_str())
        || !matches!(
            primary["verification"].as_str(),
            Some("verified" | "unverified")
        )
        || !wildcard_match(&download.filename_pattern, filename)
    {
        return Err(invalid());
    }
    let checkpoint = package_file(&root, &directory, filename)?;
    let package = manifest(&root, &directory, "package-manifest.json")?;
    if package["modelId"].as_str() != Some(entry.id.as_str())
        || package["repo"].as_str() != Some(entry.repo.as_str())
    {
        return Err(invalid());
    }
    let dependency = download
        .dependencies
        .iter()
        .find(|dep| dep.role == "mmproj" && dep.required)
        .ok_or_else(invalid)?;
    let candidates = package["files"]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .filter(|file| {
            file["role"].as_str() == Some("mmproj")
                && file["format"].as_str() == Some(dependency.format.as_str())
                && file["filename"]
                    .as_str()
                    .is_some_and(|name| wildcard_match(&dependency.filename_pattern, name))
        })
        .collect::<Vec<_>>();
    if candidates.len() != 1 || candidates[0]["verification"].as_str() == Some("failed") {
        return Err(invalid());
    }
    let projector = package_file(
        &root,
        &directory,
        candidates[0]["filename"].as_str().ok_or_else(invalid)?,
    )?;
    if checkpoint == projector {
        return Err(invalid());
    }
    Ok(LensPaths {
        checkpoint,
        projector,
    })
}

pub(super) fn select_lens_model(
    root: &Path,
    checkpoint: &Path,
    models: Vec<ModelRecord>,
) -> Result<ModelRecord, AppError> {
    let root = fs::canonicalize(root)?;
    let expected = contained_file(&root, checkpoint)?;
    models.into_iter().find(|model| {
        if !model.enabled || !matches!(model.state, ModelLifecycleState::Ready | ModelLifecycleState::Loaded)
            || model.verification.as_deref() == Some("failed") || !model.format.eq_ignore_ascii_case("gguf") { return false; }
        let path = Path::new(&model.path);
        let candidate = if path.is_absolute() { path.to_path_buf() } else { root.join(path) };
        contained_file(&root, &candidate).is_ok_and(|path| path == expected)
    }).ok_or_else(|| AppError::ModelNotFound("The exact installed OpenMindAI Lens checkpoint is not registered, enabled and ready. Validate or re-download Lens from Settings > Models.".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> (tempfile::TempDir, ModelCatalogEntry, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let entry = crate::model_catalog::entry_by_id("qwen25-vl-3b-q4km").unwrap();
        let dir = temp
            .path()
            .join(&entry.download.as_ref().unwrap().destination_dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Lens-Q4_K_M.gguf"), b"fixture checkpoint").unwrap();
        fs::write(dir.join("mmproj-Lens-Q8_0.gguf"), b"fixture projector").unwrap();
        fs::write(dir.join("model-manifest.json"), json!({"repo":entry.repo,"quantization":entry.quantization,"filename":"Lens-Q4_K_M.gguf","verification":"verified"}).to_string()).unwrap();
        fs::write(dir.join("package-manifest.json"), json!({"modelId":entry.id,"repo":entry.repo,"files":[{"role":"mmproj","format":"gguf","filename":"mmproj-Lens-Q8_0.gguf","verification":"verified"}]}).to_string()).unwrap();
        (temp, entry, dir)
    }
    fn model(path: &Path, id: &str) -> ModelRecord {
        ModelRecord {
            id: id.into(),
            name: id.into(),
            family: Some("qwen-vl".into()),
            path: path.to_string_lossy().into_owned(),
            format: "gguf".into(),
            quantization: Some("Q4_K_M".into()),
            size_bytes: 18,
            capabilities: "[\"vision\",\"ocr\"]".into(),
            context_length: Some(8192),
            preferred_backend: None,
            enabled: true,
            source_repository: None,
            verification: Some("verified".into()),
            state: ModelLifecycleState::Ready,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
    #[test]
    fn exact_lens_wins_in_every_registry_order() {
        let (temp, entry, dir) = fixture();
        let paths = lens_paths(temp.path(), &entry).unwrap();
        let other = dir.join("Other-Q8_0.gguf");
        fs::write(&other, b"other").unwrap();
        let third = dir.join("Third-F16.gguf");
        fs::write(&third, b"third").unwrap();
        let lens = model(&paths.checkpoint, "Lens");
        let a = model(&other, "AAA other Qwen");
        let z = model(&third, "ZZZ other Qwen");
        for models in [
            vec![a.clone(), lens.clone(), z.clone()],
            vec![a.clone(), z.clone(), lens.clone()],
            vec![z.clone(), a.clone(), lens.clone()],
            vec![z.clone(), lens.clone(), a.clone()],
            vec![lens.clone(), z.clone(), a.clone()],
            vec![lens.clone(), a.clone(), z.clone()],
        ] {
            assert_eq!(
                select_lens_model(temp.path(), &paths.checkpoint, models)
                    .unwrap()
                    .id,
                "Lens"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn case_distinct_checkpoints_are_not_conflated() {
        let (temp, entry, dir) = fixture();
        let paths = lens_paths(temp.path(), &entry).unwrap();
        let other = dir.join("lens-Q4_K_M.gguf");
        fs::write(&other, b"another checkpoint").unwrap();
        let models = vec![
            model(&other, "AAA lowercase"),
            model(&paths.checkpoint, "Lens"),
        ];
        assert_eq!(
            select_lens_model(temp.path(), &paths.checkpoint, models)
                .unwrap()
                .id,
            "Lens"
        );
    }
    #[test]
    fn canonical_absolute_and_relative_paths_match_without_case_folding() {
        let (temp, entry, _) = fixture();
        let paths = lens_paths(temp.path(), &entry).unwrap();
        let mut lens = model(&paths.checkpoint, "Lens");
        lens.path = paths
            .checkpoint
            .strip_prefix(fs::canonicalize(temp.path()).unwrap())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            select_lens_model(temp.path(), &paths.checkpoint, vec![lens])
                .unwrap()
                .id,
            "Lens"
        );
    }
    #[test]
    fn same_family_is_never_a_fallback_for_absent_or_unusable_lens() {
        let (temp, entry, dir) = fixture();
        let paths = lens_paths(temp.path(), &entry).unwrap();
        let other = dir.join("Other-Q8_0.gguf");
        fs::write(&other, b"other").unwrap();
        let unrelated = model(&other, "AAA Qwen");
        assert!(
            select_lens_model(temp.path(), &paths.checkpoint, vec![unrelated.clone()]).is_err()
        );
        let base = model(&paths.checkpoint, "Lens");
        let mut disabled = base.clone();
        disabled.enabled = false;
        let mut failed = base.clone();
        failed.state = ModelLifecycleState::Failed;
        let mut unverified = base.clone();
        unverified.verification = Some("failed".into());
        let mut wrong_format = base;
        wrong_format.format = "safetensors".into();
        for lens in [disabled, failed, unverified, wrong_format] {
            assert!(select_lens_model(
                temp.path(),
                &paths.checkpoint,
                vec![unrelated.clone(), lens]
            )
            .is_err());
        }
    }
    #[test]
    fn package_projector_is_used_even_if_another_projector_sorts_first() {
        let (temp, entry, dir) = fixture();
        fs::write(dir.join("mmproj-AAA-Q8_0.gguf"), b"wrong").unwrap();
        assert!(lens_paths(temp.path(), &entry)
            .unwrap()
            .projector
            .ends_with("mmproj-Lens-Q8_0.gguf"));
    }
    #[test]
    fn missing_wrong_or_ambiguous_projector_packages_are_rejected() {
        let (temp, entry, dir) = fixture();
        for package in [
            json!({"modelId":"other","repo":entry.repo,"files":[]}),
            json!({"modelId":entry.id,"repo":"other","files":[]}),
            json!({"modelId":entry.id,"repo":entry.repo,"files":[{"role":"mmproj","format":"gguf","filename":"mmproj-missing-Q8_0.gguf"}]}),
            json!({"modelId":entry.id,"repo":entry.repo,"files":[{"role":"mmproj","format":"gguf","filename":"mmproj-Lens-Q8_0.gguf"},{"role":"mmproj","format":"gguf","filename":"mmproj-Other-Q8_0.gguf"}]}),
        ] {
            fs::write(dir.join("package-manifest.json"), package.to_string()).unwrap();
            assert!(lens_paths(temp.path(), &entry).is_err());
        }
    }
    #[test]
    fn wrong_quantization_and_oversized_primary_manifests_are_rejected() {
        let (temp, entry, dir) = fixture();
        let mut primary: Value =
            serde_json::from_slice(&fs::read(dir.join("model-manifest.json")).unwrap()).unwrap();
        primary["quantization"] = json!("Q8_0");
        fs::write(dir.join("model-manifest.json"), primary.to_string()).unwrap();
        assert!(lens_paths(temp.path(), &entry).is_err());
        fs::write(
            dir.join("model-manifest.json"),
            vec![b' '; MAX_MANIFEST_BYTES as usize + 1],
        )
        .unwrap();
        assert!(lens_paths(temp.path(), &entry).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_registry_aliases_match_but_external_packages_are_rejected() {
        use std::os::unix::fs::symlink;
        let (temp, entry, dir) = fixture();
        let paths = lens_paths(temp.path(), &entry).unwrap();
        let alias = dir.join("alias.gguf");
        symlink(&paths.checkpoint, &alias).unwrap();
        assert_eq!(
            select_lens_model(temp.path(), &paths.checkpoint, vec![model(&alias, "alias")])
                .unwrap()
                .id,
            "alias"
        );
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("model.gguf"), b"outside").unwrap();
        fs::remove_file(&alias).unwrap();
        symlink(outside.path().join("model.gguf"), &alias).unwrap();
        assert!(select_lens_model(
            temp.path(),
            &paths.checkpoint,
            vec![model(&alias, "external")]
        )
        .is_err());
    }
}
