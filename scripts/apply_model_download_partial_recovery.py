from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"expected one anchor in {path}, found {count}: {old[:120]!r}")
    file.write_text(text.replace(old, new, 1), encoding="utf-8")


model_download = "src-tauri/src/model_download.rs"
model_package = "src-tauri/src/model_package.rs"

replace_once(
    model_download,
    '''        let mut existing = fs::metadata(&part_path).map(|meta| meta.len()).unwrap_or(0);\n        let mut request = self.client.get(&metadata.source_url);\n''',
    '''        let mut existing = match prepare_partial_download(\n            &part_path,\n            metadata.size_bytes,\n            metadata.sha256.as_deref(),\n        )? {\n            PartialDownloadState::Complete { verification, .. } => {\n                fs::rename(&part_path, &final_path)?;\n                tracing::info!(\n                    path = %final_path.display(),\n                    "recovered complete verified model partial without re-downloading"\n                );\n                return Ok((final_path, verification));\n            }\n            PartialDownloadState::Resume(bytes) => bytes,\n            PartialDownloadState::Fresh => 0,\n        };\n        let mut request = self.client.get(&metadata.source_url);\n''',
)

replace_once(
    model_download,
    '''        if part_size != metadata.size_bytes {\n            return Err(AppError::ModelDownloadFailed(format!(\n                "downloaded size {part_size} did not match expected {}",\n                metadata.size_bytes\n            )));\n        }\n''',
    '''        if part_size != metadata.size_bytes {\n            let _ = fs::remove_file(&part_path);\n            return Err(AppError::ModelDownloadFailed(format!(\n                "downloaded size {part_size} did not match expected {}",\n                metadata.size_bytes\n            )));\n        }\n''',
)

replace_once(
    model_download,
    '''            } else {\n                return Err(AppError::ModelChecksumFailed(format!(\n                    "expected {expected}, got {actual}"\n                )));\n            }\n''',
    '''            } else {\n                let _ = fs::remove_file(&part_path);\n                return Err(AppError::ModelChecksumFailed(format!(\n                    "expected {expected}, got {actual}"\n                )));\n            }\n''',
)

replace_once(
    model_download,
    '''fn select_sibling<'a>(\n''',
    '''#[derive(Debug, Clone, PartialEq, Eq)]\npub(crate) enum PartialDownloadState {\n    Fresh,\n    Resume(u64),\n    Complete {\n        verification: VerificationState,\n        actual_sha256: String,\n    },\n}\n\npub(crate) fn prepare_partial_download(\n    path: &Path,\n    expected_size: u64,\n    expected_sha256: Option<&str>,\n) -> Result<PartialDownloadState, AppError> {\n    let size = match fs::metadata(path) {\n        Ok(metadata) => metadata.len(),\n        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {\n            return Ok(PartialDownloadState::Fresh)\n        }\n        Err(error) => return Err(error.into()),\n    };\n\n    if size < expected_size {\n        return Ok(PartialDownloadState::Resume(size));\n    }\n    if size > expected_size {\n        tracing::warn!(\n            path = %path.display(),\n            size,\n            expected_size,\n            "discarding oversized stale partial download"\n        );\n        fs::remove_file(path)?;\n        return Ok(PartialDownloadState::Fresh);\n    }\n\n    let actual_sha256 = sha256_file(path)?;\n    let verification = match expected_sha256 {\n        Some(expected) if actual_sha256.eq_ignore_ascii_case(expected) => {\n            VerificationState::Verified\n        }\n        Some(expected) => {\n            tracing::warn!(\n                path = %path.display(),\n                expected,\n                actual = %actual_sha256,\n                "discarding complete partial download with checksum mismatch"\n            );\n            fs::remove_file(path)?;\n            return Ok(PartialDownloadState::Fresh);\n        }\n        None => VerificationState::Unverified,\n    };\n\n    Ok(PartialDownloadState::Complete {\n        verification,\n        actual_sha256,\n    })\n}\n\nfn select_sibling<'a>(\n''',
)

replace_once(
    model_download,
    '''    #[test]\n    #[ignore = "downloads the real official Qwen3 4B GGUF"]\n''',
    '''    #[test]\n    fn partial_download_resumes_when_smaller_than_expected() {\n        let temp = tempfile::tempdir().unwrap();\n        let path = temp.path().join("model.part");\n        fs::write(&path, b"partial").unwrap();\n\n        let state = prepare_partial_download(&path, 100, Some("unused")).unwrap();\n        assert_eq!(state, PartialDownloadState::Resume(7));\n        assert!(path.exists());\n    }\n\n    #[test]\n    fn partial_download_recovers_complete_matching_file() {\n        let temp = tempfile::tempdir().unwrap();\n        let path = temp.path().join("model.part");\n        fs::write(&path, b"complete payload").unwrap();\n        let expected = sha256_file(&path).unwrap();\n\n        let state = prepare_partial_download(&path, 16, Some(&expected)).unwrap();\n        assert_eq!(\n            state,\n            PartialDownloadState::Complete {\n                verification: VerificationState::Verified,\n                actual_sha256: expected,\n            }\n        );\n        assert!(path.exists());\n    }\n\n    #[test]\n    fn partial_download_discards_complete_checksum_mismatch() {\n        let temp = tempfile::tempdir().unwrap();\n        let path = temp.path().join("model.part");\n        fs::write(&path, b"complete payload").unwrap();\n\n        let state = prepare_partial_download(&path, 16, Some("wrong-checksum")).unwrap();\n        assert_eq!(state, PartialDownloadState::Fresh);\n        assert!(!path.exists());\n    }\n\n    #[test]\n    fn partial_download_discards_oversized_stale_file() {\n        let temp = tempfile::tempdir().unwrap();\n        let path = temp.path().join("model.part");\n        fs::write(&path, b"too-large").unwrap();\n\n        let state = prepare_partial_download(&path, 4, None).unwrap();\n        assert_eq!(state, PartialDownloadState::Fresh);\n        assert!(!path.exists());\n    }\n\n    #[test]\n    fn partial_download_accepts_complete_unhashed_file() {\n        let temp = tempfile::tempdir().unwrap();\n        let path = temp.path().join("model.part");\n        fs::write(&path, b"payload").unwrap();\n        let actual = sha256_file(&path).unwrap();\n\n        let state = prepare_partial_download(&path, 7, None).unwrap();\n        assert_eq!(\n            state,\n            PartialDownloadState::Complete {\n                verification: VerificationState::Unverified,\n                actual_sha256: actual,\n            }\n        );\n        assert!(path.exists());\n    }\n\n    #[test]\n    #[ignore = "downloads the real official Qwen3 4B GGUF"]\n''',
)

replace_once(
    model_package,
    '''        ensure_contained, safe_part_filename, sha256_file, validate_free_space, VerificationState,\n''',
    '''        ensure_contained, prepare_partial_download, safe_part_filename, sha256_file,\n        validate_free_space, PartialDownloadState, VerificationState,\n''',
)

replace_once(
    model_package,
    '''    let existing = fs::metadata(&part_path)\n        .map(|metadata| metadata.len())\n        .unwrap_or(0);\n    let mut request = client.get(&dependency.source_url);\n''',
    '''    let existing = match prepare_partial_download(\n        &part_path,\n        dependency.size_bytes,\n        dependency.sha256.as_deref(),\n    )? {\n        PartialDownloadState::Complete {\n            verification,\n            actual_sha256,\n        } => {\n            fs::rename(&part_path, &final_path)?;\n            tracing::info!(\n                path = %final_path.display(),\n                role = %dependency.role,\n                "recovered complete verified dependency partial without re-downloading"\n            );\n            return Ok(package_manifest_entry(\n                dependency,\n                actual_sha256,\n                verification,\n            ));\n        }\n        PartialDownloadState::Resume(bytes) => bytes,\n        PartialDownloadState::Fresh => 0,\n    };\n    let mut request = client.get(&dependency.source_url);\n''',
)

replace_once(
    model_package,
    '''    if actual_size != dependency.size_bytes {\n        return Err(AppError::ModelDownloadFailed(format!(\n            "downloaded {} dependency size {actual_size} did not match expected {}",\n            dependency.role, dependency.size_bytes\n        )));\n    }\n''',
    '''    if actual_size != dependency.size_bytes {\n        let _ = fs::remove_file(&part_path);\n        return Err(AppError::ModelDownloadFailed(format!(\n            "downloaded {} dependency size {actual_size} did not match expected {}",\n            dependency.role, dependency.size_bytes\n        )));\n    }\n''',
)

replace_once(
    model_package,
    '''        Some(expected) => {\n            return Err(AppError::ModelChecksumFailed(format!(\n                "{} dependency expected {expected}, got {actual}",\n                dependency.role\n            )))\n        }\n''',
    '''        Some(expected) => {\n            let _ = fs::remove_file(&part_path);\n            return Err(AppError::ModelChecksumFailed(format!(\n                "{} dependency expected {expected}, got {actual}",\n                dependency.role\n            )))\n        }\n''',
)

Path("scripts/apply_model_download_partial_recovery.py").unlink()
