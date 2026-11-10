use crate::app_error::AppError;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub fn reserve_directory(parent: &Path) -> Result<(PathBuf, String), AppError> {
    for _ in 0..16 {
        let id = uuid::Uuid::new_v4().to_string();
        let path = parent.join(&id);
        match fs::create_dir(&path) {
            Ok(()) => return Ok((path, id)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(AppError::internal(
        "Could not reserve an artifact output directory",
    ))
}

pub struct StagedOutput {
    final_path: PathBuf,
    pub path: PathBuf,
}
impl StagedOutput {
    pub fn new(final_path: &Path) -> Self {
        let name = final_path.file_name().unwrap_or_default().to_string_lossy();
        Self {
            final_path: final_path.to_path_buf(),
            path: final_path.with_file_name(format!(".pending-{name}")),
        }
    }
    pub fn commit_for(&self, kind: &str) -> Result<i64, AppError> {
        let mut header = Vec::new();
        fs::File::open(&self.path)?
            .take(12)
            .read_to_end(&mut header)?;
        let valid = match kind {
            "pdf" => header.starts_with(b"%PDF-"),
            "docx" => header.starts_with(b"PK\x03\x04"),
            "image" => header.starts_with(b"\x89PNG\r\n\x1a\n"),
            "audio" => header.starts_with(b"RIFF") && header.get(8..12) == Some(b"WAVE"),
            "video" => header.starts_with(b"\x1a\x45\xdf\xa3"),
            _ => false,
        };
        if !valid {
            return Err(AppError::ArtifactGenerationFailed(
                "Output signature does not match the requested artifact type".into(),
            ));
        }
        self.commit(false)
    }

    pub fn commit(&self, allow_empty: bool) -> Result<i64, AppError> {
        let metadata = fs::symlink_metadata(&self.path)?;
        if !metadata.is_file() || (!allow_empty && metadata.len() == 0) {
            return Err(AppError::ArtifactGenerationFailed(
                "Renderer did not produce a complete regular output file".into(),
            ));
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)?;
        file.sync_all()?;
        drop(file);
        // The destination belongs to our exclusively reserved UUID directory.
        if self.final_path.symlink_metadata().is_ok() {
            return Err(AppError::internal("Artifact destination already exists"));
        }
        fs::rename(&self.path, &self.final_path)?;
        #[cfg(unix)]
        fs::File::open(self.final_path.parent().unwrap())?.sync_all()?;
        Ok(metadata.len() as i64)
    }
}
impl Drop for StagedOutput {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parallel_requests_with_identical_names_have_independent_bytes_for_every_kind() {
        let root = tempfile::tempdir().unwrap();
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for extension in ["png", "webm", "wav", "pdf", "docx", "txt"] {
                for index in 0..8 {
                    let parent = root.path();
                    handles.push(scope.spawn(move || {
                        let (dir, _) = reserve_directory(parent).unwrap();
                        let final_path = dir.join(format!("same-title.{extension}"));
                        let output = StagedOutput::new(&final_path);
                        let bytes = format!("{extension}-{index}");
                        fs::write(&output.path, &bytes).unwrap();
                        assert!(!final_path.exists());
                        assert_eq!(output.commit(false).unwrap(), bytes.len() as i64);
                        assert_eq!(fs::read_to_string(&final_path).unwrap(), bytes);
                        final_path
                    }));
                }
            }
            let paths = handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<std::collections::HashSet<_>>();
            assert_eq!(paths.len(), 48);
        });
    }
    #[test]
    fn failed_or_empty_outputs_do_not_publish_or_remove_a_sibling() {
        let temp = tempfile::tempdir().unwrap();
        let (one, _) = reserve_directory(temp.path()).unwrap();
        let (two, _) = reserve_directory(temp.path()).unwrap();
        let good = two.join("same.png");
        fs::write(&good, b"keep").unwrap();
        let final_path = one.join("same.png");
        let pending;
        {
            let output = StagedOutput::new(&final_path);
            pending = output.path.clone();
            fs::write(&output.path, b"").unwrap();
            assert!(output.commit(false).is_err());
            fs::write(&output.path, b"interrupted bytes").unwrap();
        }
        assert!(!pending.exists());
        assert!(!final_path.exists());
        assert_eq!(fs::read(&good).unwrap(), b"keep");
        let output = StagedOutput::new(&final_path);
        fs::write(&output.path, b"complete").unwrap();
        fs::write(&final_path, b"already published").unwrap();
        assert!(output.commit(false).is_err());
        assert_eq!(fs::read(&final_path).unwrap(), b"already published");
    }
    #[test]
    fn signature_mismatch_cannot_be_published_as_ready_media() {
        let temp = tempfile::tempdir().unwrap();
        let final_path = temp.path().join("image.png");
        let output = StagedOutput::new(&final_path);
        fs::write(&output.path, b"not a PNG").unwrap();
        assert!(output.commit_for("image").is_err());
        assert!(!final_path.exists());
        fs::write(&output.path, b"\x89PNG\r\n\x1a\nfixture").unwrap();
        output.commit_for("image").unwrap();
        assert!(final_path.is_file());
    }

    #[test]
    fn path_guards_allow_legacy_duplicates_but_block_new_collisions() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE artifacts(id TEXT, path TEXT); INSERT INTO artifacts VALUES ('old1','legacy'),('old2','legacy');").unwrap();
        db.execute_batch(include_str!("../migrations/010_artifact_path_guards.sql"))
            .unwrap();
        assert!(db
            .execute("INSERT INTO artifacts VALUES ('new','legacy')", [])
            .is_err());
        db.execute("INSERT INTO artifacts VALUES ('new','unique')", [])
            .unwrap();
        assert!(db
            .execute("UPDATE artifacts SET path='legacy' WHERE id='new'", [])
            .is_err());
        assert_eq!(
            db.query_row("SELECT count(*) FROM artifacts", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            3
        );
    }
}
