from pathlib import Path

# 1) Expose and harden package dependency validation.
package_path = Path("src-tauri/src/model_package.rs")
package = package_path.read_text(encoding="utf-8")
marker = '''fn wildcard_match(pattern: &str, value: &str) -> bool {\n'''
validator = r'''pub(crate) fn validate_installed_dependencies(
    root: &PortableRootManager,
    entry: &ModelCatalogEntry,
    verify_hashes: bool,
) -> Result<bool, AppError> {
    let Some(download) = entry.download.as_ref() else {
        return Ok(true);
    };
    let required = download
        .dependencies
        .iter()
        .filter(|dependency| dependency.required)
        .collect::<Vec<_>>();
    if required.is_empty() {
        return Ok(true);
    }

    let model_dir = root.resolve_relative(&download.destination_dir)?;
    let manifest_path = model_dir.join("package-manifest.json");
    if !manifest_path.is_file() {
        return Ok(false);
    }
    ensure_contained(root.root(), &manifest_path)?;
    let manifest: ModelPackageManifest = match fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
    {
        Some(manifest) => manifest,
        None => return Ok(false),
    };
    if manifest.model_id != entry.id || manifest.repo != entry.repo {
        return Ok(false);
    }

    for dependency in required {
        let Some(record) = manifest.files.iter().find(|file| {
            file.role == dependency.role
                && file.format == dependency.format
                && wildcard_match(&dependency.filename_pattern, &file.filename)
        }) else {
            return Ok(false);
        };
        if record.size_bytes == 0
            || record.actual_sha256.trim().is_empty()
            || record.verification == VerificationState::Failed
        {
            return Ok(false);
        }
        if record
            .sha256
            .as_deref()
            .is_some_and(|expected| !expected.eq_ignore_ascii_case(&record.actual_sha256))
        {
            return Ok(false);
        }

        let file_path = model_dir.join(&record.filename);
        if !file_path.is_file() {
            return Ok(false);
        }
        ensure_contained(root.root(), &file_path)?;
        if fs::metadata(&file_path)?.len() != record.size_bytes {
            return Ok(false);
        }
        if verify_hashes {
            let actual = sha256_file(&file_path)?;
            if !actual.eq_ignore_ascii_case(&record.actual_sha256)
                || record
                    .sha256
                    .as_deref()
                    .is_some_and(|expected| !actual.eq_ignore_ascii_case(expected))
            {
                return Ok(false);
            }
        }
    }

    Ok(true)
}

'''
if marker not in package:
    raise SystemExit("model_package wildcard marker not found")
package = package.replace(marker, validator + marker, 1)

old_test_end = '''    fn package_pattern_selects_qwen_mmproj() {
        assert!(wildcard_match(
            "mmproj-*Q8_0*.gguf",
            "mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf"
        ));
        assert!(!wildcard_match(
            "mmproj-*Q8_0*.gguf",
            "Qwen2.5-VL-3B-Instruct-Q8_0.gguf"
        ));
    }
'''
new_test_end = old_test_end + r'''

    #[test]
    fn validates_package_manifest_and_detects_same_size_hash_tampering() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let entry = crate::model_catalog::entry_by_id("qwen25-vl-3b-q4km").unwrap();
        let download = entry.download.as_ref().unwrap();
        let model_dir = root.resolve_relative(&download.destination_dir).unwrap();
        fs::create_dir_all(&model_dir).unwrap();

        let filename = "mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf";
        let file_path = model_dir.join(filename);
        fs::write(&file_path, b"projector").unwrap();
        let actual = sha256_file(&file_path).unwrap();
        let manifest = ModelPackageManifest {
            model_id: entry.id.clone(),
            repo: entry.repo.clone(),
            files: vec![PackageFileManifest {
                role: "mmproj".to_string(),
                filename: filename.to_string(),
                format: "gguf".to_string(),
                size_bytes: 9,
                sha256: Some(actual.clone()),
                actual_sha256: actual,
                verification: VerificationState::Verified,
                source_url: "https://example.invalid/mmproj.gguf".to_string(),
            }],
        };
        fs::write(
            model_dir.join("package-manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        assert!(validate_installed_dependencies(&root, &entry, false).unwrap());
        assert!(validate_installed_dependencies(&root, &entry, true).unwrap());

        // Same-size corruption is intentionally cheap to tolerate during discovery,
        // but explicit validation must catch it by hashing the dependency.
        fs::write(&file_path, b"corrupted").unwrap();
        assert!(validate_installed_dependencies(&root, &entry, false).unwrap());
        assert!(!validate_installed_dependencies(&root, &entry, true).unwrap());
    }
'''
if old_test_end not in package:
    raise SystemExit("model_package test marker not found")
package = package.replace(old_test_end, new_test_end, 1)
package_path.write_text(package, encoding="utf-8")

# 2) Re-export the validator through the model_download module.
download_path = Path("src-tauri/src/model_download.rs")
download_text = download_path.read_text(encoding="utf-8")
old_mod = '''#[path = "model_package.rs"]\nmod model_package;\n'''
new_mod = '''#[path = "model_package.rs"]\nmod model_package;\npub(crate) use model_package::validate_installed_dependencies;\n'''
if old_mod not in download_text:
    raise SystemExit("model_download module marker not found")
download_text = download_text.replace(old_mod, new_mod, 1)
download_path.write_text(download_text, encoding="utf-8")

# 3) Make registry discovery require package metadata + size integrity, and
# explicit model validation hash all required package dependencies.
registry_path = Path("src-tauri/src/model_registry.rs")
registry = registry_path.read_text(encoding="utf-8")
old_import = '''    model_download::{validate_gguf_header, QwenModelManifest, VerificationState},\n'''
new_import = '''    model_download::{\n        validate_gguf_header, validate_installed_dependencies, QwenModelManifest,\n        VerificationState,\n    },\n'''
if old_import not in registry:
    raise SystemExit("model_registry import marker not found")
registry = registry.replace(old_import, new_import, 1)

old_ready_call = '''        if !catalog_package_ready(&canonical, manifest.as_ref()) {\n'''
new_ready_call = '''        if !catalog_package_ready(self.root, &canonical, manifest.as_ref())? {\n'''
if old_ready_call not in registry:
    raise SystemExit("catalog readiness call marker not found")
registry = registry.replace(old_ready_call, new_ready_call, 1)

old_validate = '''        let model_path = resolve_model_path(self.root, &model.path)?;\n        validate_gguf_header(&model_path, self.root)?;\n        Ok(model)\n'''
new_validate = '''        let model_path = resolve_model_path(self.root, &model.path)?;\n        validate_gguf_header(&model_path, self.root)?;\n        if let Some(manifest) = read_model_manifest(&model_path) {\n            if let Some(entry) = crate::model_catalog::load_catalog()?\n                .models\n                .into_iter()\n                .find(|entry| entry.repo == manifest.repo)\n            {\n                if !validate_installed_dependencies(self.root, &entry, true)? {\n                    return Err(AppError::ModelInvalid(format!(\n                        "{} package dependency integrity check failed",\n                        entry.name\n                    )));\n                }\n            }\n        }\n        Ok(model)\n'''
if old_validate not in registry:
    raise SystemExit("validate_model marker not found")
registry = registry.replace(old_validate, new_validate, 1)

old_ready_fn = r'''fn catalog_package_ready(model_path: &Path, manifest: Option<&QwenModelManifest>) -> bool {
    let Some(manifest) = manifest else {
        return true;
    };
    let Ok(catalog) = crate::model_catalog::load_catalog() else {
        return false;
    };
    let Some(entry) = catalog
        .models
        .into_iter()
        .find(|entry| entry.repo == manifest.repo)
    else {
        return true;
    };
    let Some(download) = entry.download else {
        return true;
    };
    let Some(parent) = model_path.parent() else {
        return false;
    };

    download
        .dependencies
        .iter()
        .filter(|dependency| dependency.required)
        .all(|dependency| directory_contains_matching_file(parent, &dependency.filename_pattern))
}

fn directory_contains_matching_file(directory: &Path, pattern: &str) -> bool {
    fs::read_dir(directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .any(|name| crate::model_catalog::wildcard_match(pattern, &name))
}
'''
new_ready_fn = r'''fn catalog_package_ready(
    root: &PortableRootManager,
    _model_path: &Path,
    manifest: Option<&QwenModelManifest>,
) -> Result<bool, AppError> {
    let Some(manifest) = manifest else {
        return Ok(true);
    };
    let catalog = crate::model_catalog::load_catalog()?;
    let Some(entry) = catalog
        .models
        .into_iter()
        .find(|entry| entry.repo == manifest.repo)
    else {
        return Ok(true);
    };
    validate_installed_dependencies(root, &entry, false)
}
'''
if old_ready_fn not in registry:
    raise SystemExit("catalog readiness function marker not found")
registry = registry.replace(old_ready_fn, new_ready_fn, 1)

old_lens_test = r'''    #[test]
    fn lens_package_requires_mmproj_dependency() {
        let temp = tempfile::tempdir().unwrap();
        let model_dir = temp.path().join("lens");
        fs::create_dir_all(&model_dir).unwrap();
        let model_path = model_dir.join("Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf");
        fs::write(&model_path, b"GGUF").unwrap();
        let manifest = QwenModelManifest {
            repo: "ggml-org/Qwen2.5-VL-3B-Instruct-GGUF".to_string(),
            repo_sha: None,
            quantization: "Q4_K_M".to_string(),
            filename: "Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf".to_string(),
            size_bytes: 4,
            sha256: None,
            actual_sha256: None,
            verification: VerificationState::Unverified,
            architecture: Some("qwen2vl".to_string()),
            context_length: Some(8192),
            chat_template_available: true,
            source_url: "https://example.invalid/model.gguf".to_string(),
            installed_at: "now".to_string(),
        };

        assert!(!catalog_package_ready(&model_path, Some(&manifest)));
        fs::write(
            model_dir.join("mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf"),
            b"GGUF",
        )
        .unwrap();
        assert!(catalog_package_ready(&model_path, Some(&manifest)));
        let capabilities = catalog_capabilities_from_manifest(&manifest).unwrap();
        assert!(capabilities.contains("vision"));
        assert!(capabilities.contains("ocr"));
    }
'''
new_lens_test = r'''    #[test]
    fn lens_package_requires_verified_package_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let model_dir = root
            .resolve_relative("models/vision/qwen2.5-vl-3b")
            .unwrap();
        fs::create_dir_all(&model_dir).unwrap();
        let model_path = model_dir.join("Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf");
        fs::write(&model_path, b"GGUF").unwrap();
        let manifest = QwenModelManifest {
            repo: "ggml-org/Qwen2.5-VL-3B-Instruct-GGUF".to_string(),
            repo_sha: None,
            quantization: "Q4_K_M".to_string(),
            filename: "Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf".to_string(),
            size_bytes: 4,
            sha256: None,
            actual_sha256: None,
            verification: VerificationState::Unverified,
            architecture: Some("qwen2vl".to_string()),
            context_length: Some(8192),
            chat_template_available: true,
            source_url: "https://example.invalid/model.gguf".to_string(),
            installed_at: "now".to_string(),
        };

        assert!(!catalog_package_ready(&root, &model_path, Some(&manifest)).unwrap());
        let dependency_name = "mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf";
        let dependency_path = model_dir.join(dependency_name);
        fs::write(&dependency_path, b"GGUF").unwrap();
        // Filename existence alone is intentionally insufficient now.
        assert!(!catalog_package_ready(&root, &model_path, Some(&manifest)).unwrap());

        let actual = crate::model_download::sha256_file(&dependency_path).unwrap();
        let package_manifest = serde_json::json!({
            "modelId": "qwen25-vl-3b-q4km",
            "repo": "ggml-org/Qwen2.5-VL-3B-Instruct-GGUF",
            "files": [{
                "role": "mmproj",
                "filename": dependency_name,
                "format": "gguf",
                "sizeBytes": 4,
                "sha256": null,
                "actualSha256": actual,
                "verification": "unverified",
                "sourceUrl": "https://example.invalid/mmproj.gguf"
            }]
        });
        fs::write(
            model_dir.join("package-manifest.json"),
            serde_json::to_string_pretty(&package_manifest).unwrap(),
        )
        .unwrap();
        assert!(catalog_package_ready(&root, &model_path, Some(&manifest)).unwrap());
        let capabilities = catalog_capabilities_from_manifest(&manifest).unwrap();
        assert!(capabilities.contains("vision"));
        assert!(capabilities.contains("ocr"));
    }
'''
if old_lens_test not in registry:
    raise SystemExit("lens package registry test marker not found")
registry = registry.replace(old_lens_test, new_lens_test, 1)
registry_path.write_text(registry, encoding="utf-8")
