use std::{fs, path::Path};

use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    app_error::AppError,
    database::Database,
    model_download::{validate_gguf_header, QwenModelManifest, VerificationState},
    portable_root::PortableRootManager,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRecord {
    pub id: String,
    pub name: String,
    pub family: Option<String>,
    pub path: String,
    pub format: String,
    pub quantization: Option<String>,
    pub size_bytes: i64,
    pub capabilities: String,
    pub context_length: Option<i64>,
    pub preferred_backend: Option<String>,
    pub enabled: bool,
    pub source_repository: Option<String>,
    pub verification: Option<String>,
    pub state: ModelLifecycleState,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub enum ModelLifecycleState {
    NotInstalled,
    Downloading,
    Installed,
    Validating,
    Ready,
    Loading,
    Loaded,
    Unloading,
    Failed,
}

pub struct ModelRegistry<'a> {
    database: &'a Database,
    root: &'a PortableRootManager,
}

impl<'a> ModelRegistry<'a> {
    pub fn new(database: &'a Database, root: &'a PortableRootManager) -> Self {
        Self { database, root }
    }

    pub fn discover_gguf_models(&self) -> Result<Vec<ModelRecord>, AppError> {
        let mut discovered = Vec::new();
        scan_directory(&self.root.models_dir(), &mut discovered)?;

        for model_path in discovered {
            self.register_gguf(&model_path)?;
        }

        self.list_models()
    }

    pub fn list_models(&self) -> Result<Vec<ModelRecord>, AppError> {
        let mut statement = self.database.connection().prepare(
            "SELECT id, name, family, path, format, quantization, size_bytes, capabilities,
                    context_length, preferred_backend, enabled, created_at, updated_at
             FROM model_registry
             ORDER BY name ASC",
        )?;
        let rows = statement.query_map([], map_model)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
    }

    pub fn register_gguf(&self, path: &Path) -> Result<(), AppError> {
        let canonical = fs::canonicalize(path)?;
        let canonical_root = fs::canonicalize(self.root.root())?;
        if !canonical.starts_with(&canonical_root) {
            return Err(AppError::ModelInvalid(
                "model must be inside OpenMindAI Root".to_string(),
            ));
        }

        let relative_path = canonical
            .strip_prefix(&canonical_root)
            .map_err(|_| {
                AppError::ModelInvalid("model must be inside OpenMindAI Root".to_string())
            })?
            .to_string_lossy()
            .replace('\\', "/");
        let manifest = read_model_manifest(&canonical);
        if !catalog_package_ready(&canonical, manifest.as_ref()) {
            self.database.connection().execute(
                "DELETE FROM model_registry WHERE path = ?1 OR path = ?2",
                params![relative_path, canonical.display().to_string()],
            )?;
            tracing::warn!(
                path = %canonical.display(),
                "model package is incomplete; skipping registry activation"
            );
            return Ok(());
        }

        let existing: Option<String> = self
            .database
            .connection()
            .query_row(
                "SELECT id FROM model_registry WHERE path = ?1 OR path = ?2",
                params![relative_path, canonical.display().to_string()],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(id) = existing {
            let name = manifest.as_ref().map(display_name_from_manifest);
            let capabilities = manifest
                .as_ref()
                .and_then(catalog_capabilities_from_manifest);
            self.database.connection().execute(
                "UPDATE model_registry
                 SET path = ?1, name = COALESCE(?2, name), capabilities = COALESCE(?3, capabilities), updated_at = ?4
                 WHERE id = ?5",
                params![
                    relative_path,
                    name,
                    capabilities,
                    Utc::now().to_rfc3339(),
                    id
                ],
            )?;
            return Ok(());
        }

        validate_gguf_header(&canonical, self.root)?;
        let metadata = fs::metadata(&canonical)?;
        let id = stable_model_id(&relative_path);
        let now = Utc::now().to_rfc3339();
        let name = manifest
            .as_ref()
            .map(display_name_from_manifest)
            .or_else(|| {
                canonical
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .map(ToString::to_string)
            })
            .unwrap_or_else(|| "GGUF model".to_string());
        let family = manifest
            .as_ref()
            .and_then(|manifest| manifest.architecture.clone())
            .or_else(|| {
                canonical
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .map(infer_family)
            });
        let quantization = manifest
            .as_ref()
            .map(|manifest| manifest.quantization.clone())
            .or_else(|| {
                canonical
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .and_then(infer_quantization)
            });
        let context_length = manifest
            .as_ref()
            .and_then(|manifest| manifest.context_length.map(|value| value as i64));
        let capabilities = manifest
            .as_ref()
            .and_then(catalog_capabilities_from_manifest)
            .unwrap_or_else(|| {
                if manifest
                    .as_ref()
                    .is_some_and(|manifest| manifest.chat_template_available)
                {
                    "[\"chat\",\"thinking\"]".to_string()
                } else {
                    "[\"chat\"]".to_string()
                }
            });

        self.database.connection().execute(
            "INSERT INTO model_registry
             (id, name, family, path, format, quantization, size_bytes, capabilities,
              context_length, preferred_backend, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'gguf', ?5, ?6, ?7, ?8, 'vulkan', 1, ?9, ?10)",
            params![
                id,
                name,
                family,
                relative_path,
                quantization,
                metadata.len() as i64,
                capabilities,
                context_length,
                now,
                now
            ],
        )?;
        Ok(())
    }

    pub fn validate_model(&self, id: &str) -> Result<ModelRecord, AppError> {
        let model = self
            .list_models()?
            .into_iter()
            .find(|model| model.id == id)
            .ok_or_else(|| AppError::ModelNotFound(id.to_string()))?;
        let model_path = resolve_model_path(self.root, &model.path)?;
        validate_gguf_header(&model_path, self.root)?;
        Ok(model)
    }

    pub fn remove_by_path_prefix(&self, relative_prefix: &str) -> Result<(), AppError> {
        let prefix = relative_prefix
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_string();
        self.database.connection().execute(
            "DELETE FROM model_registry WHERE path = ?1 OR path LIKE ?2",
            params![prefix, format!("{prefix}/%")],
        )?;
        Ok(())
    }
}

fn read_model_manifest(model_path: impl AsRef<Path>) -> Option<QwenModelManifest> {
    let manifest_path = model_path.as_ref().parent()?.join("model-manifest.json");
    let content = fs::read_to_string(manifest_path).ok()?;
    serde_json::from_str(&content).ok()
}

fn catalog_capabilities_from_manifest(manifest: &QwenModelManifest) -> Option<String> {
    let entry = crate::model_catalog::load_catalog()
        .ok()?
        .models
        .into_iter()
        .find(|entry| entry.repo == manifest.repo)?;
    serde_json::to_string(&entry.capabilities).ok()
}

fn catalog_package_ready(model_path: &Path, manifest: Option<&QwenModelManifest>) -> bool {
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

fn resolve_model_path(
    root: &PortableRootManager,
    path: &str,
) -> Result<std::path::PathBuf, AppError> {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        Ok(candidate.to_path_buf())
    } else {
        root.resolve_relative(path)
    }
}

fn display_name_from_manifest(manifest: &QwenModelManifest) -> String {
    crate::model_catalog::load_catalog()
        .ok()
        .and_then(|catalog| {
            catalog
                .models
                .into_iter()
                .find(|entry| entry.repo == manifest.repo)
        })
        .map(|entry| entry.name)
        .unwrap_or_else(|| {
            manifest
                .filename
                .strip_suffix(".gguf")
                .unwrap_or(&manifest.filename)
                .to_string()
        })
}

fn infer_family(name: &str) -> String {
    name.split(['-', '_'])
        .next()
        .unwrap_or("gguf")
        .to_lowercase()
}

fn infer_quantization(name: &str) -> Option<String> {
    ["Q4_K_M", "Q5_0", "Q5_K_M", "Q6_K", "Q8_0", "F16"]
        .iter()
        .find(|quant| {
            name.to_ascii_uppercase()
                .contains(&quant.to_ascii_uppercase())
        })
        .map(|quant| quant.to_string())
}

fn verification_for(path: &str) -> Option<String> {
    let manifest = model_manifest_for_path(path)?;
    Some(
        match manifest.verification {
            VerificationState::Verified => "verified",
            VerificationState::Unverified => "unverified",
            VerificationState::Failed => "failed",
        }
        .to_string(),
    )
}

fn model_manifest_for_path(path: &str) -> Option<QwenModelManifest> {
    read_model_manifest(Path::new(path)).or_else(|| {
        PortableRootManager::resolve()
            .ok()
            .and_then(|root| resolve_model_path(&root, path).ok())
            .and_then(read_model_manifest)
    })
}

fn scan_directory(directory: &Path, found: &mut Vec<std::path::PathBuf>) -> Result<(), AppError> {
    if !directory.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            scan_directory(&path, found)?;
        } else if path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gguf"))
            && !path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().starts_with("mmproj-"))
        {
            found.push(path);
        }
    }
    Ok(())
}

fn stable_model_id(path: &str) -> String {
    let digest = Sha256::digest(path.as_bytes());
    format!("gguf-{:x}", digest)[0..21].to_string()
}

fn map_model(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModelRecord> {
    let path: String = row.get(3)?;
    Ok(ModelRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        family: row.get(2)?,
        path: path.clone(),
        format: row.get(4)?,
        quantization: row.get(5)?,
        size_bytes: row.get(6)?,
        capabilities: row.get(7)?,
        context_length: row.get(8)?,
        preferred_backend: row.get(9)?,
        enabled: row.get::<_, i64>(10)? != 0,
        source_repository: model_manifest_for_path(&path).map(|manifest| manifest.repo),
        verification: verification_for(&path),
        state: ModelLifecycleState::Ready,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_valid_gguf_without_duplicates() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let mut content = vec![0_u8; 33 * 1024 * 1024];
        content[0..4].copy_from_slice(b"GGUF");
        fs::write(root.models_dir().join("llm").join("tiny.gguf"), content).unwrap();
        let database = Database::in_memory().unwrap();
        let registry = ModelRegistry::new(&database, &root);
        assert_eq!(registry.discover_gguf_models().unwrap().len(), 1);
        assert_eq!(registry.discover_gguf_models().unwrap().len(), 1);
    }

    #[test]
    fn invalid_gguf_returns_model_invalid() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let path = root.models_dir().join("llm").join("broken.gguf");
        fs::write(&path, b"not gguf").unwrap();
        let database = Database::in_memory().unwrap();
        let registry = ModelRegistry::new(&database, &root);
        let err = registry.register_gguf(&path).unwrap_err();
        assert!(matches!(err, AppError::ModelInvalid(_)));
    }

    #[test]
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

    #[test]
    #[ignore = "registers the real downloaded Qwen GGUF in the portable database"]
    fn real_qwen_model_registers_in_portable_database() {
        let root = PortableRootManager::resolve().unwrap();
        root.ensure_directories().unwrap();
        let database = Database::open(root.database_path()).unwrap();
        let registry = ModelRegistry::new(&database, &root);
        let models = registry.discover_gguf_models().unwrap();
        let qwen = models
            .into_iter()
            .find(|model| model.source_repository.as_deref() == Some("Qwen/Qwen3-4B-GGUF"))
            .expect("Qwen model should be registered");

        assert_eq!(qwen.name, "OpenMindAI Core");
        assert_eq!(qwen.family.as_deref(), Some("qwen3"));
        assert_eq!(qwen.quantization.as_deref(), Some("Q4_K_M"));
        assert_eq!(qwen.size_bytes, 2_497_280_256);
        assert_eq!(qwen.verification.as_deref(), Some("verified"));
        assert_eq!(
            qwen.source_repository.as_deref(),
            Some("Qwen/Qwen3-4B-GGUF")
        );
        assert!(!qwen.capabilities.contains("vision"));
        assert!(!qwen.capabilities.contains("embedding"));
        assert!(!qwen.capabilities.contains("image"));
    }
}
