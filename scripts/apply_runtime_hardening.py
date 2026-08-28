from pathlib import Path


PATH = Path("src-tauri/src/diffusion_runtime.rs")
text = PATH.read_text(encoding="utf-8")


def replace_once(old: str, new: str) -> None:
    global text
    if old not in text:
        raise SystemExit(f"anchor not found: {old[:180]!r}")
    text = text.replace(old, new, 1)


replace_once(
    '''const RELEASES_URL: &str =
    "https://api.github.com/repos/leejet/stable-diffusion.cpp/releases/latest";
const RUNTIME_ROOT: &str = "runtimes/diffusion/stable-diffusion.cpp";''',
    '''const PINNED_RUNTIME_TAG: &str = "master-829-0a565f2";
const RELEASE_URL: &str =
    "https://api.github.com/repos/leejet/stable-diffusion.cpp/releases/tags/master-829-0a565f2";
const RUNTIME_ROOT: &str = "runtimes/diffusion/stable-diffusion.cpp";''',
)

replace_once(
    '''struct DiffusionRuntimeManifest {
    version: String,
    backend: BackendKind,
    source_url: String,
    archive_sha256: String,
    cli_path: String,
    installed_at: String,
}''',
    '''struct DiffusionRuntimeManifest {
    version: String,
    backend: BackendKind,
    source_url: String,
    archive_sha256: String,
    cli_path: String,
    #[serde(default)]
    cli_sha256: String,
    #[serde(default)]
    cli_size: u64,
    installed_at: String,
}''',
)

replace_once(
    '''    if let Some(manifest) = load_manifest(root)? {
        let cli = root.resolve_relative(&manifest.cli_path)?;
        if cli.is_file() && preferred_backend.is_none_or(|backend| *backend == manifest.backend) {
            return Ok(manifest);
        }
    }''',
    '''    if let Some(manifest) = load_manifest(root)? {
        if runtime_manifest_is_reusable(root, &manifest, preferred_backend)? {
            return Ok(manifest);
        }
        tracing::warn!(
            version = %manifest.version,
            backend = ?manifest.backend,
            "installed diffusion runtime is stale or failed integrity validation; reinstalling pinned runtime"
        );
    }''',
)

replace_once(
    '''    let release = client
        .get(RELEASES_URL)
        .header(header::USER_AGENT, "OpenMindAI/2")
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|error| AppError::RuntimeInstallFailed(error.to_string()))?
        .error_for_status()
        .map_err(|error| AppError::RuntimeInstallFailed(error.to_string()))?
        .json::<GithubRelease>()
        .await
        .map_err(|error| AppError::RuntimeInstallFailed(error.to_string()))?;

    let (backend, asset) = candidates''',
    '''    let release = client
        .get(RELEASE_URL)
        .header(header::USER_AGENT, "OpenMindAI/2")
        .timeout(Duration::from_secs(60))
        .send()
        .await
        .map_err(|error| AppError::RuntimeInstallFailed(error.to_string()))?
        .error_for_status()
        .map_err(|error| AppError::RuntimeInstallFailed(error.to_string()))?
        .json::<GithubRelease>()
        .await
        .map_err(|error| AppError::RuntimeInstallFailed(error.to_string()))?;
    if release.tag_name != PINNED_RUNTIME_TAG {
        return Err(AppError::RuntimeInstallFailed(format!(
            "pinned stable-diffusion.cpp release endpoint returned unexpected tag {}; expected {PINNED_RUNTIME_TAG}",
            release.tag_name
        )));
    }

    let (backend, asset) = candidates''',
)

replace_once(
    '''                "latest stable-diffusion.cpp release {} has no compatible runtime asset",
                release.tag_name''',
    '''                "pinned stable-diffusion.cpp release {} has no compatible runtime asset",
                release.tag_name''',
)

replace_once(
    '''    let relative_cli = cli_path
        .strip_prefix(root.root())
        .map_err(|_| {
            AppError::RuntimeInstallFailed("runtime CLI escaped OpenMindAI Root".to_string())
        })?
        .to_string_lossy()
        .replace('\\\\', "/");
    let manifest = DiffusionRuntimeManifest {
        version: version.to_string(),
        backend,
        source_url: asset.browser_download_url.clone(),
        archive_sha256: actual_sha256,
        cli_path: relative_cli,
        installed_at: Utc::now().to_rfc3339(),
    };''',
    '''    let relative_cli = cli_path
        .strip_prefix(root.root())
        .map_err(|_| {
            AppError::RuntimeInstallFailed("runtime CLI escaped OpenMindAI Root".to_string())
        })?
        .to_string_lossy()
        .replace('\\\\', "/");
    let cli_size = fs::metadata(&cli_path)?.len();
    if cli_size == 0 {
        return Err(AppError::RuntimeInstallFailed(
            "stable-diffusion.cpp runtime CLI is empty after extraction".to_string(),
        ));
    }
    let cli_sha256 = sha256_file(&cli_path)?;
    let manifest = DiffusionRuntimeManifest {
        version: version.to_string(),
        backend,
        source_url: asset.browser_download_url.clone(),
        archive_sha256: actual_sha256,
        cli_path: relative_cli,
        cli_sha256,
        cli_size,
        installed_at: Utc::now().to_rfc3339(),
    };''',
)

replace_once(
    '''fn load_manifest(root: &PortableRootManager) -> Result<Option<DiffusionRuntimeManifest>, AppError> {''',
    '''fn runtime_manifest_is_reusable(
    root: &PortableRootManager,
    manifest: &DiffusionRuntimeManifest,
    preferred_backend: Option<&BackendKind>,
) -> Result<bool, AppError> {
    if manifest.version != PINNED_RUNTIME_TAG {
        return Ok(false);
    }
    if preferred_backend.is_some_and(|backend| *backend != manifest.backend) {
        return Ok(false);
    }
    if manifest.cli_size == 0 || manifest.cli_sha256.len() != 64 {
        return Ok(false);
    }

    let cli = root.resolve_relative(&manifest.cli_path)?;
    if !cli.is_file() {
        return Ok(false);
    }
    let metadata = match fs::metadata(&cli) {
        Ok(metadata) => metadata,
        Err(error) => {
            tracing::warn!(%error, path = %cli.display(), "could not stat installed diffusion runtime CLI");
            return Ok(false);
        }
    };
    if metadata.len() != manifest.cli_size {
        return Ok(false);
    }
    let actual_sha256 = match sha256_file(&cli) {
        Ok(digest) => digest,
        Err(error) => {
            tracing::warn!(%error, path = %cli.display(), "could not verify installed diffusion runtime CLI");
            return Ok(false);
        }
    };
    Ok(actual_sha256.eq_ignore_ascii_case(&manifest.cli_sha256))
}

fn load_manifest(root: &PortableRootManager) -> Result<Option<DiffusionRuntimeManifest>, AppError> {''',
)

replace_once(
    '''    #[test]
    fn safe_component_removes_path_syntax() {
        assert_eq!(safe_component("master/829:abc"), "master-829-abc");
    }
}''',
    '''    #[test]
    fn safe_component_removes_path_syntax() {
        assert_eq!(safe_component("master/829:abc"), "master-829-abc");
    }

    #[test]
    fn runtime_manifest_reuse_requires_pinned_version_backend_and_cli_integrity() {
        let temp = tempfile::tempdir().unwrap();
        let root = PortableRootManager::from_root(temp.path().join("OpenMindAI"));
        root.ensure_directories().unwrap();
        let cli_relative = if cfg!(target_os = "windows") {
            "runtimes/diffusion/stable-diffusion.cpp/cpu/master-829-0a565f2/sd-cli.exe"
        } else {
            "runtimes/diffusion/stable-diffusion.cpp/cpu/master-829-0a565f2/sd-cli"
        };
        let cli = root.resolve_relative(cli_relative).unwrap();
        fs::create_dir_all(cli.parent().unwrap()).unwrap();
        fs::write(&cli, b"known-good-runtime").unwrap();
        let cli_sha256 = sha256_file(&cli).unwrap();
        let cli_size = fs::metadata(&cli).unwrap().len();
        let mut manifest = DiffusionRuntimeManifest {
            version: PINNED_RUNTIME_TAG.to_string(),
            backend: BackendKind::Cpu,
            source_url: "https://example.invalid/runtime.zip".to_string(),
            archive_sha256: "a".repeat(64),
            cli_path: cli_relative.to_string(),
            cli_sha256,
            cli_size,
            installed_at: "2026-08-28T00:00:00Z".to_string(),
        };

        assert!(runtime_manifest_is_reusable(&root, &manifest, Some(&BackendKind::Cpu)).unwrap());
        assert!(!runtime_manifest_is_reusable(&root, &manifest, Some(&BackendKind::Vulkan)).unwrap());

        manifest.version = "master-828-old".to_string();
        assert!(!runtime_manifest_is_reusable(&root, &manifest, Some(&BackendKind::Cpu)).unwrap());
        manifest.version = PINNED_RUNTIME_TAG.to_string();

        fs::write(&cli, b"tampered-runtime!!").unwrap();
        assert_eq!(fs::metadata(&cli).unwrap().len(), cli_size);
        assert!(!runtime_manifest_is_reusable(&root, &manifest, Some(&BackendKind::Cpu)).unwrap());
    }

    #[test]
    fn legacy_manifest_without_cli_integrity_is_invalidated_for_reinstall() {
        let legacy = format!(
            r#"{{"version":"{PINNED_RUNTIME_TAG}","backend":"cpu","source_url":"https://example.invalid/runtime.zip","archive_sha256":"{}","cli_path":"runtimes/diffusion/sd-cli","installed_at":"2026-08-28T00:00:00Z"}}"#,
            "b".repeat(64)
        );
        let manifest: DiffusionRuntimeManifest = serde_json::from_str(&legacy).unwrap();
        assert!(manifest.cli_sha256.is_empty());
        assert_eq!(manifest.cli_size, 0);
    }
}''',
)

PATH.write_text(text, encoding="utf-8")
Path("scripts/apply_runtime_hardening.py").unlink(missing_ok=True)
