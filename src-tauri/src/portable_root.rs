#[path = "portable_output.rs"]
mod portable_output;

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::app_error::AppError;

const MARKER_FILE: &str = "openmindai.marker";
const INSTALL_CONFIG_FILE: &str = "install.json";
pub const CURRENT_SCHEMA_VERSION: u32 = 8;
pub const DEFAULT_PROFILE_NAME: &str = "openmindai";
/// Rough initial footprint for a first-run install: llama.cpp runtime +
/// Qwen3 4B Q4_K_M model + database + safety margin. Used only to preview
/// "is there probably enough room" on the setup wizard's Storage screen —
/// the actual model download separately re-validates free space for real
/// (see `model_download.rs`'s `validate_free_space`).
pub const RECOMMENDED_INITIAL_STORAGE_BYTES: u64 = 6 * 1024 * 1024 * 1024;

/// `std::fs::canonicalize` prefixes Windows paths with the extended-length
/// verbatim prefix (`\\?\C:\...`), which `sysinfo`'s plain `C:\`-style
/// mount points never match via [`Path::starts_with`]. Strip it before
/// comparing against a disk's mount point.
pub(crate) fn strip_windows_verbatim_prefix(path: &Path) -> PathBuf {
    match path.to_str() {
        Some(raw) => PathBuf::from(raw.strip_prefix(r"\\?\").unwrap_or(raw)),
        None => path.to_path_buf(),
    }
}

/// Finds available disk space for `path`, walking up to the nearest
/// existing ancestor when `path` itself doesn't exist yet (e.g. a setup
/// wizard previewing a not-yet-created root).
pub fn available_bytes_for_path(path: &Path) -> Option<u64> {
    let mut probe = path.to_path_buf();
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }
    let canonical = strip_windows_verbatim_prefix(&fs::canonicalize(&probe).ok()?);
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .iter()
        .filter(|disk| canonical.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().as_os_str().len())
        .map(|disk| disk.available_space())
}

/// Non-destructive writability probe: writes and immediately removes a temp
/// file if `path` exists, otherwise recurses to the nearest existing
/// ancestor. Deliberately does not create `path` itself — previewing a
/// candidate setup location shouldn't leave stray folders behind.
pub fn preview_writable(path: &Path) -> bool {
    if path.exists() {
        // Never truncate or delete a pre-existing file (or follow a planted
        // symlink) merely to check whether a storage location is writable.
        let probe = path.join(format!(".openmindai-write-test-{}", uuid::Uuid::new_v4()));
        return match fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
            Ok(file) => {
                drop(file);
                fs::remove_file(&probe).is_ok()
            }
            Err(_) => false,
        };
    }
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => preview_writable(parent),
        _ => false,
    }
}

#[derive(Debug, Clone)]
pub struct PortableRootManager {
    root: PathBuf,
    mode: RootMode,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableRootInfo {
    pub root: String,
    pub mode: RootMode,
    pub database_path: String,
    pub models_dir: String,
    pub logs_dir: String,
    pub writable: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RootMode {
    DevelopmentEnv,
    SavedInstallation,
    PortableMarker,
    DevelopmentFallback,
}

/// Explicit first-run progress, persisted alongside [`InstallConfig`] so the
/// setup wizard can resume at the right step instead of restarting from
/// "welcome" if the app is closed mid-setup. `RuntimeReady`/`ModelReady` are
/// set independently (whichever finishes first) before `Completed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SetupState {
    StorageChosen,
    ProfileChosen,
    RootCreated,
    RuntimeReady,
    ModelReady,
    Completed,
}

/// Small, source-of-truth pointer persisted outside `OPENMINDAI_ROOT` (in the
/// user's local app-data directory) so a production install remembers which
/// root/profile the user chose during first-run setup without needing the
/// `OPENMINDAI_ROOT` environment variable or a portable marker file.
///
/// Also used, with `installation_complete: false`, to remember a *pending*
/// choice made mid-wizard (storage path / profile name picked, but setup not
/// finished yet) — see `SetupState`. New fields use `#[serde(default)]` so an
/// already-installed user's existing file on disk keeps parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallConfig {
    pub product: String,
    pub app_version: String,
    pub root: String,
    pub schema_version: u32,
    pub installation_complete: bool,
    pub database: String,
    pub runtime_ready: bool,
    pub model_ready: bool,
    #[serde(default)]
    pub setup_state: Option<SetupState>,
}

impl InstallConfig {
    /// Whether this install finished provisioning far enough to run without the
    /// setup wizard. `installation_complete` alone is *not* enough: it is
    /// committed the moment the root directory is created (see `complete_setup`),
    /// long before the runtime and model are downloaded. A setup interrupted
    /// after that point — or whose runtime/model download never finished — leaves
    /// `installation_complete: true` with a non-`Completed` `setup_state`, and
    /// trusting the flag alone drops the user into an app that can only answer
    /// "server failed" on every chat. `setup_state` is the authoritative,
    /// resumable progress marker, so it decides completion here.
    ///
    /// Legacy installs written before `setup_state` existed have `None`; those
    /// predate the resumable-setup feature and were only ever written on a fully
    /// finished install, so they count as provisioned to avoid forcing existing
    /// users back through the wizard.
    pub fn is_fully_provisioned(&self) -> bool {
        self.installation_complete
            && matches!(self.setup_state, None | Some(SetupState::Completed))
    }

    /// Builds the resume descriptor for a setup that still needs finishing.
    fn pending_setup(&self) -> PendingSetup {
        PendingSetup {
            root: self.root.clone(),
            profile_name: self
                .database
                .strip_suffix(".db")
                .unwrap_or(&self.database)
                .to_string(),
            state: self.setup_state.unwrap_or(SetupState::StorageChosen),
        }
    }
}

/// A partially-completed setup the wizard can resume from — surfaced to the
/// frontend only while `setup_required` is still true.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingSetup {
    pub root: String,
    pub profile_name: String,
    pub state: SetupState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallationStatus {
    pub setup_required: bool,
    pub reason: Option<String>,
    pub root: Option<String>,
    pub pending: Option<PendingSetup>,
}

/// Sanitizes a user-supplied local profile/database name down to
/// `[a-z0-9-_]`, falling back to [`DEFAULT_PROFILE_NAME`] when nothing usable
/// remains (e.g. an empty or purely-symbolic input).
pub fn sanitize_profile_name(input: &str) -> String {
    let cleaned: String = input
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if cleaned.is_empty() {
        DEFAULT_PROFILE_NAME.to_string()
    } else {
        cleaned.to_lowercase()
    }
}

fn install_config_path() -> Option<PathBuf> {
    // Test/advanced-override seam: lets tests point at a temp directory
    // instead of the real per-user AppData location.
    if let Ok(value) = env::var("OPENMINDAI_INSTALL_CONFIG") {
        if !value.trim().is_empty() {
            return Some(PathBuf::from(value));
        }
    }
    dirs::config_local_dir().map(|dir| dir.join("OpenMindAI").join(INSTALL_CONFIG_FILE))
}

/// Read the saved pointer, recovering its backup when necessary. Only a
/// genuinely absent primary and backup represent a fresh installation.
pub fn load_installation() -> Result<Option<InstallConfig>, AppError> {
    let path = install_config_path()
        .ok_or_else(|| AppError::internal("could not determine local app data directory"))?;
    crate::installation_file::load(&path)
}

pub fn save_installation(config: &InstallConfig) -> Result<(), AppError> {
    let path = install_config_path()
        .ok_or_else(|| AppError::internal("could not determine local app data directory"))?;
    crate::installation_file::save(&path, config)
}

/// Computes whether the first-run setup wizard should be shown. Development
/// roots (env var override or portable marker file) never require setup —
/// only a production build with neither a saved installation nor a dev
/// signal falls back needing one.
pub fn installation_status(manager: &PortableRootManager) -> Result<InstallationStatus, AppError> {
    Ok(match manager.mode {
        RootMode::SavedInstallation => {
            // The pointer is committed, but `complete_setup` writes
            // `installation_complete: true` the instant the root directory
            // exists — before the runtime and model are downloaded. A setup
            // interrupted after that (or whose download never finished) leaves a
            // half-provisioned install that can only answer "server failed" on
            // chat. Re-surface the wizard to finish provisioning against the same
            // (already-opened) root instead of trusting the flag alone.
            let saved = load_installation()?;
            match saved {
                Some(config) if !config.is_fully_provisioned() => InstallationStatus {
                    setup_required: true,
                    reason: Some(
                        "installation incomplete: runtime or model not provisioned".to_string(),
                    ),
                    root: Some(manager.root.display().to_string()),
                    pending: Some(config.pending_setup()),
                },
                _ => InstallationStatus {
                    setup_required: false,
                    reason: None,
                    root: Some(manager.root.display().to_string()),
                    pending: None,
                },
            }
        }
        RootMode::DevelopmentEnv | RootMode::PortableMarker => InstallationStatus {
            setup_required: false,
            reason: Some("development root detected".to_string()),
            root: Some(manager.root.display().to_string()),
            pending: None,
        },
        RootMode::DevelopmentFallback => {
            let saved = load_installation()?;
            let has_saved_config = saved.as_ref().is_some_and(|c| c.installation_complete);
            let pending = saved
                .filter(|config| !config.installation_complete)
                .map(|config| config.pending_setup());
            InstallationStatus {
                setup_required: !has_saved_config,
                reason: Some("no installation configured".to_string()),
                root: None,
                pending,
            }
        }
    })
}

impl PortableRootManager {
    pub fn resolve() -> Result<Self, AppError> {
        if let Ok(value) = env::var("OPENMINDAI_ROOT") {
            if !value.trim().is_empty() {
                return Ok(Self {
                    root: PathBuf::from(value),
                    mode: RootMode::DevelopmentEnv,
                });
            }
        }

        if let Some(config) = load_installation()? {
            if config.installation_complete {
                let manager = Self {
                    root: PathBuf::from(&config.root),
                    mode: RootMode::SavedInstallation,
                };
                manager.validate_saved_installation(&config.database)?;
                return Ok(manager);
            }
        }

        if let Some(root) = find_marker_root(env::current_exe().ok().as_deref()) {
            return Ok(Self {
                root,
                mode: RootMode::PortableMarker,
            });
        }

        Ok(Self {
            root: env::current_dir()?.join("OpenMindAI"),
            mode: RootMode::DevelopmentFallback,
        })
    }

    /// Builds a manager for an explicit root, bypassing environment/marker
    /// resolution. Used both by tests and by first-run setup, which needs to
    /// operate on the user's chosen directory before any installation
    /// pointer has been saved.
    pub fn from_root(root: PathBuf) -> Self {
        Self {
            root,
            mode: RootMode::DevelopmentFallback,
        }
    }

    pub fn mode(&self) -> &RootMode {
        &self.mode
    }

    /// `database_path` is the *actually opened* database file, not
    /// necessarily `self.database_path()`'s unparameterized default — a
    /// production install opens a profile-named database
    /// (`database_path_for_profile`), and callers must pass that path
    /// through rather than let this silently report the wrong file.
    pub fn info(&self, database_path: &Path) -> Result<PortableRootInfo, AppError> {
        Ok(PortableRootInfo {
            root: self.root.display().to_string(),
            mode: self.mode.clone(),
            database_path: database_path.display().to_string(),
            models_dir: self.models_dir().display().to_string(),
            logs_dir: self.logs_dir().display().to_string(),
            writable: self.validate_root().is_ok(),
        })
    }

    pub fn ensure_directories(&self) -> Result<(), AppError> {
        // Resolve the root once, then create every directory through a
        // capability-relative handle. A concurrent symlink replacement of an
        // intermediate directory must not redirect initialization outside it.
        fs::create_dir_all(&self.root)?;
        let root = cap_std::fs::Dir::open_ambient_dir(
            &self.root,
            cap_std::ambient_authority(),
        )?;
        for relative in REQUIRED_DIRECTORIES {
            root.create_dir_all(relative)?;
        }
        self.validate_root()?;
        Ok(())
    }

    // Startup must not create directories or a database for an existing install.
    fn validate_saved_installation(&self, database: &str) -> Result<(), AppError> {
        let unavailable = || {
            AppError::RootNotWritable(format!(
            "Saved installation at {} is unavailable or incomplete. Reconnect the drive and check access to data/database/{}. No fallback data store was opened.",
            self.root.display(), database
        ))
        };
        let root = fs::canonicalize(&self.root).map_err(|_| unavailable())?;
        if !root.is_dir()
            || database.is_empty()
            || database.contains(['/', '\\', ':'])
            || database == "."
            || database == ".."
        {
            return Err(unavailable());
        }
        let path = root.join("data/database").join(database);
        let canonical = fs::canonicalize(&path).map_err(|_| unavailable())?;
        if !canonical.starts_with(&root) || !canonical.is_file() {
            return Err(unavailable());
        }
        fs::File::open(&canonical).map_err(|_| unavailable())?;
        let probe = root.join(format!(".openmind-startup-{}.tmp", uuid::Uuid::new_v4()));
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .map_err(|_| unavailable())?;
        drop(file);
        fs::remove_file(probe).map_err(|_| unavailable())?;
        Ok(())
    }

    pub fn validate_root(&self) -> Result<(), AppError> {
        fs::create_dir_all(&self.root)?;
        // A fixed .write-test name could be a symlink to unrelated user data.
        // Use an unpredictable exclusive file so validation cannot overwrite it.
        let probe = self.root.join(format!(".write-test-{}", uuid::Uuid::new_v4()));
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .map_err(|error| AppError::RootNotWritable(error.to_string()))?;
        drop(file);
        fs::remove_file(&probe)
            .map_err(|error| AppError::RootNotWritable(error.to_string()))?;
        Ok(())
    }

    pub fn resolve_relative(&self, relative: impl AsRef<Path>) -> Result<PathBuf, AppError> {
        portable_output::resolve(&self.root, relative.as_ref())
    }

    /// Create a fresh text output through directory capabilities. Existing
    /// files are never truncated, and actual opens stay beneath the root.
    #[cfg(test)]
    pub fn write_new_relative(
        &self,
        relative: impl AsRef<Path>,
        bytes: &[u8],
    ) -> Result<(), AppError> {
        portable_output::write_new(&self.root, relative.as_ref(), bytes)
    }

    pub fn database_path(&self) -> PathBuf {
        self.root
            .join("data")
            .join("database")
            .join("openmind_ai.db")
    }

    /// Database path for a sanitized local profile name, e.g. `"shagor-ai"`
    /// -> `data/database/shagor-ai.db`. Callers should sanitize untrusted
    /// input with [`sanitize_profile_name`] first.
    pub fn database_path_for_profile(&self, profile: &str) -> PathBuf {
        self.root
            .join("data")
            .join("database")
            .join(format!("{profile}.db"))
    }

    pub fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn find_marker_root(start: Option<&Path>) -> Option<PathBuf> {
    let mut cursor = start.and_then(Path::parent)?;
    loop {
        if cursor.join(MARKER_FILE).is_file() {
            return Some(cursor.to_path_buf());
        }
        cursor = cursor.parent()?;
    }
}

const REQUIRED_DIRECTORIES: &[&str] = &[
    "models/llm",
    "models/vision",
    "models/embedding",
    "models/image",
    "runtimes/llama",
    "runtimes/llama/manifests",
    "runtimes/llama/cpu",
    "runtimes/llama/cuda12",
    "runtimes/llama/cuda13",
    "runtimes/llama/vulkan",
    "runtimes/llama/sycl",
    "runtimes/llama/hip",
    "runtimes/llama/active",
    "runtimes/diffusion",
    "runtimes/diffusion/stable-diffusion.cpp",
    "runtimes/python",
    "runtimes/node",
    "runtimes/git",
    "data/database",
    "data/memory",
    "data/vectors",
    "data/indexes",
    "datasets",
    "datasets/openmindai",
    "workspaces",
    "knowledge",
    "generated/images",
    "generated/files",
    "generated/exports",
    "cache",
    "temp",
    "logs",
    "config",
    "backups",
];

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    // `pub(crate)` so other modules' tests that resolve the real
    // `OPENMINDAI_ROOT` (e.g. `runtime::tests::real_staged_runtime_validates_when_present`)
    // can serialize against these env-var-mutating tests instead of racing
    // them under `cargo test`'s default parallel execution.
    pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn creates_required_directories() {
        let temp = tempfile::tempdir().unwrap();
        let manager = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        manager.ensure_directories().unwrap();
        assert!(manager.database_path().parent().unwrap().is_dir());
        assert!(manager.models_dir().join("llm").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn ensure_directories_rejects_external_symlink_without_writing_outside() {
        use std::os::unix::fs::symlink;
        let base = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        fs::create_dir_all(&root).unwrap();
        symlink(outside.path(), root.join("generated")).unwrap();
        let manager = PortableRootManager::from_root(root);
        assert!(manager.ensure_directories().is_err());
        assert!(!outside.path().join("images").exists());
        assert!(!outside.path().join("files").exists());
        assert!(!outside.path().join("exports").exists());
    }

    #[cfg(unix)]
    #[test]
    fn ensure_directories_accepts_contained_symlink() {
        use std::os::unix::fs::symlink;
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        fs::create_dir_all(root.join("real-generated")).unwrap();
        symlink("real-generated", root.join("generated")).unwrap();
        let manager = PortableRootManager::from_root(root.clone());
        manager.ensure_directories().unwrap();
        assert!(root.join("real-generated/images").is_dir());
    }

    #[test]
    fn rejects_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let manager = PortableRootManager::from_root(temp.path().to_path_buf());
        assert!(manager.resolve_relative("../outside").is_err());
        assert!(manager.resolve_relative("models/llm").is_ok());
    }

    #[test]
    fn resolves_env_override() {
        let _guard = ENV_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let previous = env::var_os("OPENMINDAI_ROOT");
        env::set_var("OPENMINDAI_ROOT", temp.path());

        let manager = PortableRootManager::resolve().unwrap();
        assert_eq!(manager.root(), temp.path());

        if let Some(previous) = previous {
            env::set_var("OPENMINDAI_ROOT", previous);
        } else {
            env::remove_var("OPENMINDAI_ROOT");
        }
    }

    #[test]
    fn resolves_expected_env_root_when_provided() {
        let _guard = ENV_LOCK.lock().unwrap();
        let Some(expected) = env::var_os("OPENMINDAI_EXPECTED_ROOT") else {
            return;
        };
        let previous = env::var_os("OPENMINDAI_ROOT");
        env::set_var("OPENMINDAI_ROOT", &expected);

        let manager = PortableRootManager::resolve().unwrap();
        assert_eq!(manager.root(), Path::new(&expected));

        if let Some(previous) = previous {
            env::set_var("OPENMINDAI_ROOT", previous);
        } else {
            env::remove_var("OPENMINDAI_ROOT");
        }
    }

    struct EnvRestore {
        root: Option<std::ffi::OsString>,
        install_config: Option<std::ffi::OsString>,
    }

    impl EnvRestore {
        fn capture() -> Self {
            let restore = Self {
                root: env::var_os("OPENMINDAI_ROOT"),
                install_config: env::var_os("OPENMINDAI_INSTALL_CONFIG"),
            };
            env::remove_var("OPENMINDAI_ROOT");
            restore
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            match self.root.take() {
                Some(value) => env::set_var("OPENMINDAI_ROOT", value),
                None => env::remove_var("OPENMINDAI_ROOT"),
            }
            match self.install_config.take() {
                Some(value) => env::set_var("OPENMINDAI_INSTALL_CONFIG", value),
                None => env::remove_var("OPENMINDAI_INSTALL_CONFIG"),
            }
        }
    }

    #[test]
    fn resolves_saved_installation_when_valid() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture();

        let root_temp = tempfile::tempdir().unwrap();
        let root_dir = root_temp.path().join("My AI Root");
        let config_temp = tempfile::tempdir().unwrap();
        let config_path = config_temp.path().join("install.json");
        env::set_var("OPENMINDAI_INSTALL_CONFIG", &config_path);

        let config = InstallConfig {
            product: "OpenMindAI".to_string(),
            app_version: "1.0.0".to_string(),
            root: root_dir.display().to_string(),
            schema_version: CURRENT_SCHEMA_VERSION,
            installation_complete: true,
            database: "openmindai.db".to_string(),
            runtime_ready: false,
            model_ready: false,
            setup_state: None,
        };
        fs::create_dir_all(root_dir.join("data/database")).unwrap();
        fs::write(root_dir.join("data/database/openmindai.db"), b"fixture").unwrap();
        save_installation(&config).unwrap();

        let manager = PortableRootManager::resolve().unwrap();
        assert_eq!(manager.mode(), &RootMode::SavedInstallation);
        assert_eq!(manager.root(), root_dir.as_path());
    }

    #[test]
    fn saved_installation_resumes_when_setup_not_completed() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture();

        let root_temp = tempfile::tempdir().unwrap();
        let root_dir = root_temp.path().join("My AI Root");
        let config_temp = tempfile::tempdir().unwrap();
        let config_path = config_temp.path().join("install.json");
        env::set_var("OPENMINDAI_INSTALL_CONFIG", &config_path);

        // Mirrors the install.json reproduced after a fresh install whose model
        // download never finished: the pointer is committed and the runtime is
        // ready, but `setup_state` never reached `Completed`. Trusting
        // `installation_complete` here would drop the user straight into chat,
        // where every message fails with "server failed".
        let config = InstallConfig {
            product: "OpenMindAI".to_string(),
            app_version: "4.0.0".to_string(),
            root: root_dir.display().to_string(),
            schema_version: CURRENT_SCHEMA_VERSION,
            installation_complete: true,
            database: "shagor-ai.db".to_string(),
            runtime_ready: true,
            model_ready: false,
            setup_state: Some(SetupState::RuntimeReady),
        };
        fs::create_dir_all(root_dir.join("data/database")).unwrap();
        fs::write(root_dir.join("data/database/shagor-ai.db"), b"fixture").unwrap();
        save_installation(&config).unwrap();

        let manager = PortableRootManager::resolve().unwrap();
        assert_eq!(manager.mode(), &RootMode::SavedInstallation);

        let status = installation_status(&manager).unwrap();
        assert!(status.setup_required);
        let pending = status.pending.expect("incomplete install should resume");
        assert_eq!(pending.root, root_dir.display().to_string());
        assert_eq!(pending.profile_name, "shagor-ai");
        assert_eq!(pending.state, SetupState::RuntimeReady);
        // The chosen root stays active so provisioning resumes in place,
        // preserving any data already written there.
        assert_eq!(status.root, Some(root_dir.display().to_string()));
    }

    #[test]
    fn legacy_saved_installation_without_setup_state_stays_complete() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture();

        let root_temp = tempfile::tempdir().unwrap();
        let root_dir = root_temp.path().join("My AI Root");
        let config_temp = tempfile::tempdir().unwrap();
        let config_path = config_temp.path().join("install.json");
        env::set_var("OPENMINDAI_INSTALL_CONFIG", &config_path);

        // A real install written before `setup_state` existed: no recorded
        // progress, yet genuinely finished. It must not be forced back through
        // the wizard.
        let config = InstallConfig {
            product: "OpenMindAI".to_string(),
            app_version: "1.0.0".to_string(),
            root: root_dir.display().to_string(),
            schema_version: CURRENT_SCHEMA_VERSION,
            installation_complete: true,
            database: "openmindai.db".to_string(),
            runtime_ready: false,
            model_ready: false,
            setup_state: None,
        };
        fs::create_dir_all(root_dir.join("data/database")).unwrap();
        fs::write(root_dir.join("data/database/openmindai.db"), b"fixture").unwrap();
        save_installation(&config).unwrap();

        let manager = PortableRootManager::resolve().unwrap();
        assert_eq!(manager.mode(), &RootMode::SavedInstallation);

        let status = installation_status(&manager).unwrap();
        assert!(!status.setup_required);
        assert!(status.pending.is_none());
    }

    #[test]
    fn refuses_fallback_when_saved_root_is_unreachable() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture();

        let temp = tempfile::tempdir().unwrap();
        let blocker_file = temp.path().join("blocker");
        fs::write(&blocker_file, b"not a directory").unwrap();
        // A path whose parent component is a regular file can never be
        // created as a directory — a deterministic "unreachable root".
        let unreachable_root = blocker_file.join("OpenMindAI");

        let config_temp = tempfile::tempdir().unwrap();
        let config_path = config_temp.path().join("install.json");
        env::set_var("OPENMINDAI_INSTALL_CONFIG", &config_path);

        let config = InstallConfig {
            product: "OpenMindAI".to_string(),
            app_version: "1.0.0".to_string(),
            root: unreachable_root.display().to_string(),
            schema_version: CURRENT_SCHEMA_VERSION,
            installation_complete: true,
            database: "openmindai.db".to_string(),
            runtime_ready: false,
            model_ready: false,
            setup_state: None,
        };
        save_installation(&config).unwrap();

        let error = PortableRootManager::resolve().unwrap_err().to_string();
        assert!(error.contains("Reconnect the drive"));
    }

    #[test]
    fn missing_drive_and_database_are_not_recreated_and_reconnection_can_retry() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("disconnected-drive/selected-root");
        let pointer = temp.path().join("install.json");
        env::set_var("OPENMINDAI_INSTALL_CONFIG", &pointer);
        let config = InstallConfig {
            product: "OpenMindAI".into(),
            app_version: "4.0.0".into(),
            root: root.display().to_string(),
            schema_version: CURRENT_SCHEMA_VERSION,
            installation_complete: true,
            database: "chosen.db".into(),
            runtime_ready: true,
            model_ready: true,
            setup_state: Some(SetupState::Completed),
        };
        save_installation(&config).unwrap();
        let original = fs::read(&pointer).unwrap();
        assert!(PortableRootManager::resolve().is_err());
        assert!(!root.exists());
        fs::create_dir_all(&root).unwrap();
        assert!(PortableRootManager::resolve().is_err());
        assert!(!root.join("data").exists());
        fs::create_dir_all(root.join("data/database")).unwrap();
        fs::write(root.join("data/database/chosen.db"), b"existing data").unwrap();
        let manager = PortableRootManager::resolve().unwrap();
        assert_eq!(manager.mode(), &RootMode::SavedInstallation);
        assert_eq!(
            fs::read(root.join("data/database/chosen.db")).unwrap(),
            b"existing data"
        );
        assert_eq!(fs::read(&pointer).unwrap(), original);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn read_only_saved_root_is_rejected_without_writing_a_probe() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        fs::create_dir_all(root.join("data/database")).unwrap();
        fs::write(root.join("data/database/profile.db"), b"existing").unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o500)).unwrap();
        let manager = PortableRootManager::from_root(root.clone());
        let result = manager.validate_saved_installation("profile.db");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        assert_eq!(
            fs::read(root.join("data/database/profile.db")).unwrap(),
            b"existing"
        );
    }

    #[test]
    fn ignores_incomplete_saved_installation() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture();

        let root_temp = tempfile::tempdir().unwrap();
        let root_dir = root_temp.path().join("My AI Root");
        let config_temp = tempfile::tempdir().unwrap();
        let config_path = config_temp.path().join("install.json");
        env::set_var("OPENMINDAI_INSTALL_CONFIG", &config_path);

        let config = InstallConfig {
            product: "OpenMindAI".to_string(),
            app_version: "1.0.0".to_string(),
            root: root_dir.display().to_string(),
            schema_version: CURRENT_SCHEMA_VERSION,
            installation_complete: false,
            database: "openmindai.db".to_string(),
            runtime_ready: false,
            model_ready: false,
            setup_state: None,
        };
        save_installation(&config).unwrap();

        let manager = PortableRootManager::resolve().unwrap();
        assert_ne!(manager.mode(), &RootMode::SavedInstallation);
    }

    #[test]
    fn corrupt_installation_does_not_resolve_to_a_fresh_root() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("install.json");
        env::set_var("OPENMINDAI_INSTALL_CONFIG", &path);
        fs::write(&path, b"{partial").unwrap();
        let error = PortableRootManager::resolve().unwrap_err().to_string();
        assert!(error.contains("Restore a valid install.json"));
        assert!(
            installation_status(&PortableRootManager::from_root(temp.path().join("unused")))
                .is_err()
        );
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[test]
    fn old_install_json_without_setup_state_still_loads() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture();

        let config_temp = tempfile::tempdir().unwrap();
        let config_path = config_temp.path().join("install.json");
        env::set_var("OPENMINDAI_INSTALL_CONFIG", &config_path);

        // Mirrors a real install.json written before `setup_state` existed —
        // no such field present at all.
        let legacy_json = r#"{
            "product": "OpenMindAI",
            "appVersion": "0.1.0",
            "root": "G:\\portable ai",
            "schemaVersion": 1,
            "installationComplete": true,
            "database": "openmindai.db",
            "runtimeReady": false,
            "modelReady": false
        }"#;
        fs::write(&config_path, legacy_json).unwrap();

        let config = load_installation()
            .unwrap()
            .expect("legacy install.json should still parse");
        assert!(config.installation_complete);
        assert_eq!(config.setup_state, None);
    }

    #[test]
    fn surfaces_pending_setup_for_resumability() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture();

        let config_temp = tempfile::tempdir().unwrap();
        let config_path = config_temp.path().join("install.json");
        env::set_var("OPENMINDAI_INSTALL_CONFIG", &config_path);

        let config = InstallConfig {
            product: "OpenMindAI".to_string(),
            app_version: "1.0.0".to_string(),
            root: "D:\\OpenMindAI".to_string(),
            schema_version: CURRENT_SCHEMA_VERSION,
            installation_complete: false,
            database: "shagor-ai.db".to_string(),
            runtime_ready: false,
            model_ready: false,
            setup_state: Some(SetupState::ProfileChosen),
        };
        save_installation(&config).unwrap();

        let manager = PortableRootManager::from_root(PathBuf::from("unused"));
        let status = installation_status(&manager).unwrap();
        assert!(status.setup_required);
        let pending = status.pending.expect("pending setup should be surfaced");
        assert_eq!(pending.root, "D:\\OpenMindAI");
        assert_eq!(pending.profile_name, "shagor-ai");
        assert_eq!(pending.state, SetupState::ProfileChosen);
    }

    #[test]
    fn completed_installation_has_no_pending_setup() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture();

        let config_temp = tempfile::tempdir().unwrap();
        let config_path = config_temp.path().join("install.json");
        env::set_var("OPENMINDAI_INSTALL_CONFIG", &config_path);

        let config = InstallConfig {
            product: "OpenMindAI".to_string(),
            app_version: "1.0.0".to_string(),
            root: "D:\\OpenMindAI".to_string(),
            schema_version: CURRENT_SCHEMA_VERSION,
            installation_complete: true,
            database: "openmindai.db".to_string(),
            runtime_ready: true,
            model_ready: true,
            setup_state: Some(SetupState::Completed),
        };
        save_installation(&config).unwrap();

        let manager = PortableRootManager::from_root(PathBuf::from("unused"));
        let status = installation_status(&manager).unwrap();
        assert!(!status.setup_required);
        assert!(status.pending.is_none());
    }

    #[test]
    fn handles_root_paths_with_spaces() {
        let temp = tempfile::tempdir().unwrap();
        let root_dir = temp.path().join("My AI Root");
        let manager = PortableRootManager::from_root(root_dir.clone());
        manager.ensure_directories().unwrap();

        let db_path = manager.database_path_for_profile("shagor-ai");
        assert!(db_path.starts_with(&root_dir));
        assert_eq!(db_path.file_name().unwrap(), "shagor-ai.db");
        assert!(db_path.parent().unwrap().is_dir());
    }

    #[test]
    fn reports_available_bytes_for_existing_and_missing_paths() {
        let temp = tempfile::tempdir().unwrap();
        let existing = available_bytes_for_path(temp.path());
        assert!(existing.is_some());

        let not_yet_created = temp.path().join("My AI Root").join("nested");
        let via_ancestor = available_bytes_for_path(&not_yet_created);
        assert!(via_ancestor.is_some());
    }

    #[test]
    fn preview_writable_does_not_create_the_target_path() {
        let temp = tempfile::tempdir().unwrap();
        let candidate = temp.path().join("My AI Root");
        assert!(!candidate.exists());

        assert!(preview_writable(&candidate));
        assert!(
            !candidate.exists(),
            "preview must not create the target directory"
        );
    }

    #[test]
    fn sanitizes_profile_names() {
        assert_eq!(sanitize_profile_name("shagor-ai"), "shagor-ai");
        assert_eq!(sanitize_profile_name("  My Profile!! "), "myprofile");
        assert_eq!(sanitize_profile_name(""), DEFAULT_PROFILE_NAME);
        assert_eq!(sanitize_profile_name("!!!"), DEFAULT_PROFILE_NAME);
    }
}

#[cfg(test)]
mod writability_probe_regression_tests {
    use super::*;

    #[test]
    fn validation_does_not_modify_existing_probe_file() {
        let root = tempfile::tempdir().unwrap();
        let sentinel = root.path().join(".write-test");
        fs::write(&sentinel, b"important user data").unwrap();
        PortableRootManager::from_root(root.path().to_path_buf())
            .validate_root()
            .unwrap();
        assert_eq!(fs::read(&sentinel).unwrap(), b"important user data");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn preview_does_not_modify_existing_probe_file() {
        let root = tempfile::tempdir().unwrap();
        let sentinel = root.path().join(".openmindai-write-test");
        fs::write(&sentinel, b"keep").unwrap();
        assert!(preview_writable(root.path()));
        assert_eq!(fs::read(&sentinel).unwrap(), b"keep");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_probe_cannot_overwrite_external_file() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("protected");
        fs::write(&target, b"protected").unwrap();
        symlink(&target, root.path().join(".write-test")).unwrap();
        symlink(&target, root.path().join(".openmindai-write-test")).unwrap();
        assert!(preview_writable(root.path()));
        PortableRootManager::from_root(root.path().to_path_buf())
            .validate_root()
            .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"protected");
    }
}
