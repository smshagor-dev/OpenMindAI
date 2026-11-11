use std::{fs, path::Path};

use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    app_error::AppError,
    database::Database,
    model_download::{
        validate_gguf_header, validate_installed_dependencies, QwenModelManifest, VerificationState,
    },
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
        let discovered =
            crate::model_discovery::scan_models(&self.root.models_dir(), self.root.root())?;
        for model_path in discovered.files {
            match self.register_gguf(&model_path) {
                Ok(()) => {}
                // A database failure is not an individual bad filesystem entry.
                Err(error @ AppError::DatabaseInitFailed(_)) => return Err(error),
                Err(error) => tracing::warn!(path = %model_path.display(), %error,
                    "skipping model that could not be registered during discovery"),
            }
        }

        self.list_models()
    }

    pub fn list_models(&self) -> Result<Vec<ModelRecord>, AppError> {
        let mut statement = self.database.connection().prepare(
            "SELECT id, name, family, path, format, quantization, size_bytes, capabilities,
                    context_length, preferred_backend, enabled, created_at, updated_at
             FROM model_registry
             WHERE format <> 'cloud'
             ORDER BY name ASC",
        )?;
        let rows = statement.query_map([], map_model)?;
        let mut models = rows.collect::<Result<Vec<_>, _>>()?;
        for model in &mut models {
            self.refresh_availability(model);
        }
        Ok(models)
    }

    fn refresh_availability(&self, model: &mut ModelRecord) {
        let inspect = || -> Result<Option<(u64, Option<QwenModelManifest>)>, AppError> {
            let path = resolve_model_path(self.root, &model.path)?;
            let metadata = match fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            if !metadata.is_file() {
                return Err(AppError::ModelInvalid("model is not a regular file".into()));
            }
            validate_gguf_header(&path, self.root)?;
            let manifest = read_model_manifest(&path);
            if !catalog_package_ready(self.root, &path, manifest.as_ref())? {
                return Err(AppError::ModelInvalid("model package is incomplete".into()));
            }
            Ok(Some((metadata.len(), manifest)))
        };
        match inspect() {
            Ok(Some((size, manifest))) => {
                model.size_bytes = i64::try_from(size).unwrap_or(i64::MAX);
                model.source_repository = manifest.as_ref().map(|record| record.repo.clone());
                model.verification = manifest.map(|record| {
                    match record.verification {
                        VerificationState::Verified => "verified",
                        VerificationState::Unverified => "unverified",
                        VerificationState::Failed => "failed",
                    }
                    .to_string()
                });
            }
            Ok(None) => {
                model.enabled = false;
                model.state = ModelLifecycleState::NotInstalled;
            }
            Err(_) => {
                model.enabled = false;
                model.state = ModelLifecycleState::Failed;
            }
        }
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
        if !catalog_package_ready(self.root, &canonical, manifest.as_ref())? {
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

        if let Some(id) = existing {
            self.database.connection().execute(
                "UPDATE model_registry SET path=?1, name=COALESCE(?2,name), family=?3,
                 quantization=?4, size_bytes=?5, capabilities=?6, context_length=?7, updated_at=?8
                 WHERE id=?9",
                params![
                    relative_path,
                    manifest.as_ref().map(display_name_from_manifest),
                    family,
                    quantization,
                    metadata.len() as i64,
                    capabilities,
                    context_length,
                    now,
                    id
                ],
            )?;
            return Ok(());
        }

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
        if let Some(manifest) = read_model_manifest(&model_path) {
            if let Some(entry) = crate::model_catalog::load_catalog()?
                .models
                .into_iter()
                .find(|entry| entry.repo == manifest.repo)
            {
                if !validate_installed_dependencies(self.root, &entry, true)? {
                    return Err(AppError::ModelInvalid(format!(
                        "{} package dependency integrity check failed",
                        entry.name
                    )));
                }
            }
        }
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

fn catalog_package_ready(
    root: &PortableRootManager,
    _model_path: &Path,
    manifest: Option<&QwenModelManifest>,
) -> Result<bool, AppError> {
    let Some(manifest) = manifest else {
        return Ok(true);
    };
    if manifest.verification == VerificationState::Failed {
        return Ok(false);
    }
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
        source_repository: None,
        verification: None,
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

    fn write_sparse_gguf(path: &Path) {
        use std::io::Write;
        let mut file = fs::File::create(path).unwrap();
        file.write_all(b"GGUF").unwrap();
        file.set_len(33 * 1024 * 1024).unwrap();
    }

    #[test]
    fn discovery_skips_invalid_files_and_registers_unrelated_valid_models() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        fs::write(root.models_dir().join("broken.gguf"), b"invalid").unwrap();
        write_sparse_gguf(&root.models_dir().join("valid.gguf"));
        let database = Database::in_memory().unwrap();
        let registry = ModelRegistry::new(&database, &root);
        let models = registry.discover_gguf_models().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].path, "models/valid.gguf");
        assert_eq!(registry.discover_gguf_models().unwrap().len(), 1);
    }

    #[test]
    fn discovery_still_reports_database_failures() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        write_sparse_gguf(&root.models_dir().join("valid.gguf"));
        let database = Database::in_memory().unwrap();
        database
            .connection()
            .pragma_update(None, "query_only", true)
            .unwrap();
        let error = ModelRegistry::new(&database, &root)
            .discover_gguf_models()
            .unwrap_err();
        assert!(matches!(error, AppError::DatabaseInitFailed(_)));
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
    fn fixture_manifest(
        path: &Path,
        size: u64,
        architecture: &str,
        quantization: &str,
        context: u64,
    ) {
        let record = QwenModelManifest {
            repo: "fixture/custom".into(),
            repo_sha: None,
            quantization: quantization.into(),
            filename: path.file_name().unwrap().to_string_lossy().into_owned(),
            size_bytes: size,
            sha256: None,
            actual_sha256: None,
            verification: VerificationState::Unverified,
            architecture: Some(architecture.into()),
            context_length: Some(context),
            chat_template_available: true,
            source_url: "https://example.invalid/fixture".into(),
            installed_at: "fixture".into(),
        };
        fs::write(
            path.parent().unwrap().join("model-manifest.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn replacement_refreshes_metadata_and_size_without_resetting_preferences() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let path = root.models_dir().join("llm/fixture.gguf");
        write_sparse_gguf(&path);
        fixture_manifest(&path, 33 * 1024 * 1024, "first", "Q4_K_M", 8192);
        let db = Database::in_memory().unwrap();
        let registry = ModelRegistry::new(&db, &root);
        let original = registry.discover_gguf_models().unwrap().remove(0);
        db.connection()
            .execute(
                "UPDATE model_registry SET enabled=0, preferred_backend='cpu' WHERE id=?1",
                [&original.id],
            )
            .unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(35 * 1024 * 1024)
            .unwrap();
        fixture_manifest(&path, 35 * 1024 * 1024, "second", "Q8_0", 32768);
        // Even a list read before rediscovery must not plan with the old size.
        assert_eq!(
            registry.list_models().unwrap()[0].size_bytes,
            35 * 1024 * 1024
        );
        let updated = registry.discover_gguf_models().unwrap().remove(0);
        assert_eq!(updated.id, original.id);
        assert_eq!(updated.created_at, original.created_at);
        assert_eq!(updated.size_bytes, 35 * 1024 * 1024);
        assert_eq!(updated.family.as_deref(), Some("second"));
        assert_eq!(updated.quantization.as_deref(), Some("Q8_0"));
        assert_eq!(updated.context_length, Some(32768));
        assert!(!updated.enabled);
        assert_eq!(updated.preferred_backend.as_deref(), Some("cpu"));
        assert_eq!(updated.source_repository.as_deref(), Some("fixture/custom"));
        assert_eq!(updated.verification.as_deref(), Some("unverified"));
        use crate::hardware::{BackendProfile, CpuProfile, HardwareProfile, MemoryProfile};
        let hardware = HardwareProfile {
            operating_system: "fixture".into(),
            architecture: "x86_64".into(),
            cpu: CpuProfile {
                name: "fixture".into(),
                physical_cores: Some(4),
                logical_threads: 8,
            },
            memory: MemoryProfile {
                total_bytes: 16 * 1024 * 1024 * 1024,
                available_bytes: 16 * 1024 * 1024 * 1024,
            },
            gpus: vec![],
            primary_gpu: None,
            recommended_inference_gpu: None,
            backends: BackendProfile {
                cpu: true,
                cuda: false,
                vulkan: false,
                sycl: false,
                hip: false,
                metal: false,
            },
            detection_complete: true,
        };
        let plan = crate::launch_planner::ModelLaunchPlanner::plan(&updated, &hardware, 8080);
        assert_eq!(plan.estimated_model_bytes, 35 * 1024 * 1024);
    }

    #[test]
    fn deleted_model_stays_in_history_but_is_unavailable_until_restored() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let path = root.models_dir().join("llm/kept.gguf");
        write_sparse_gguf(&path);
        let db = Database::in_memory().unwrap();
        let registry = ModelRegistry::new(&db, &root);
        let original = registry.discover_gguf_models().unwrap().remove(0);
        let chats = crate::chat::ChatRepository::new(&db);
        let chat = chats.create_conversation(Some("keep history")).unwrap();
        chats
            .set_active_model(&chat.id, Some(&original.id))
            .unwrap();
        fs::remove_file(&path).unwrap();
        let missing = registry.discover_gguf_models().unwrap().remove(0);
        assert_eq!(missing.id, original.id);
        assert!(!missing.enabled);
        assert_eq!(missing.state, ModelLifecycleState::NotInstalled);
        assert_eq!(
            db.connection()
                .query_row(
                    "SELECT enabled FROM model_registry WHERE id=?1",
                    [&original.id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert_eq!(
            chats
                .find_conversation(&chat.id)
                .unwrap()
                .active_model_id
                .as_deref(),
            Some(original.id.as_str())
        );
        write_sparse_gguf(&path);
        let restored = registry.discover_gguf_models().unwrap().remove(0);
        assert!(restored.enabled);
        assert_eq!(restored.id, original.id);
        assert_eq!(restored.state, ModelLifecycleState::Ready);
    }

    #[test]
    fn invalid_replacement_is_not_activated_and_does_not_erase_preferences() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let path = root.models_dir().join("llm/kept.gguf");
        write_sparse_gguf(&path);
        let db = Database::in_memory().unwrap();
        let registry = ModelRegistry::new(&db, &root);
        let original = registry.discover_gguf_models().unwrap().remove(0);
        fs::write(&path, b"not a model").unwrap();
        let invalid = registry.discover_gguf_models().unwrap().remove(0);
        assert_eq!(invalid.id, original.id);
        assert!(!invalid.enabled);
        assert_eq!(invalid.state, ModelLifecycleState::Failed);
        assert!(registry.register_gguf(&path).is_err());
        write_sparse_gguf(&path);
        assert!(registry.discover_gguf_models().unwrap()[0].enabled);
    }
    #[test]
    fn incomplete_catalog_package_retains_the_existing_registry_row() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let path = root.models_dir().join("llm/fixture.gguf");
        write_sparse_gguf(&path);
        fixture_manifest(&path, 33 * 1024 * 1024, "fixture", "Q4_K_M", 8192);
        let db = Database::in_memory().unwrap();
        let registry = ModelRegistry::new(&db, &root);
        let original = registry.discover_gguf_models().unwrap().remove(0);
        let manifest_path = path.parent().unwrap().join("model-manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["repo"] = serde_json::json!("ggml-org/Qwen2.5-VL-3B-Instruct-GGUF");
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let unavailable = registry.discover_gguf_models().unwrap().remove(0);
        assert_eq!(unavailable.id, original.id);
        assert!(!unavailable.enabled);
        assert_eq!(unavailable.state, ModelLifecycleState::Failed);
        assert_eq!(
            db.connection()
                .query_row(
                    "SELECT count(*) FROM model_registry WHERE id=?1",
                    [&original.id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        fixture_manifest(&path, 33 * 1024 * 1024, "fixture", "Q4_K_M", 8192);
        let restored = registry.discover_gguf_models().unwrap().remove(0);
        assert_eq!(restored.id, original.id);
        assert!(restored.enabled);
    }
}
