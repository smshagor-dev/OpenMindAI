#[path = "docx_integrity.rs"]
mod docx_integrity;
#[path = "pdf_integrity.rs"]
mod pdf_integrity;

use crate::app_error::AppError;
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use std::{
    fs,
    io::{self, Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

/// Create beneath an acquired root capability; never create via a checked path.
pub fn reserve_directory(root: &Path, relative: &Path) -> Result<(Dir, String), AppError> {
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err(AppError::internal(
            "Artifact output path must stay beneath its root",
        ));
    }
    let root = Dir::open_ambient_dir(root, ambient_authority())?;
    root.create_dir_all(relative)?;
    let parent = root.open_dir(relative)?;
    for _ in 0..16 {
        let id = uuid::Uuid::new_v4().to_string();
        match parent.create_dir(&id) {
            Ok(()) => return Ok((parent.open_dir(&id)?, id)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(AppError::internal(
        "Could not reserve an artifact output directory",
    ))
}

pub struct StagedOutput {
    directory: Dir,
    published: AtomicBool,
    final_name: String,
    pub path: PathBuf,
    _staging: tempfile::TempDir,
}
impl StagedOutput {
    pub fn from_reserved(directory: Dir, name: &str) -> Result<Self, AppError> {
        if name.is_empty()
            || name.contains(['/', '\\', ':'])
            || !matches!(
                Path::new(name).components().next(),
                Some(Component::Normal(_))
            )
        {
            return Err(AppError::internal(
                "Artifact filename must be one normal component",
            ));
        }
        let staging = tempfile::Builder::new()
            .prefix("openmind-render-")
            .tempdir()?;
        Ok(Self {
            directory,
            published: AtomicBool::new(false),
            final_name: name.to_owned(),
            path: staging.path().join(name),
            _staging: staging,
        })
    }

    fn source(&self, allow_empty: bool) -> Result<fs::File, AppError> {
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
        Ok(file)
    }

    pub fn commit_for(&self, kind: &str) -> Result<i64, AppError> {
        let mut source = self.source(false)?;
        let mut header = Vec::new();
        (&mut source).take(12).read_to_end(&mut header)?;
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
        if kind == "pdf" {
            pdf_integrity::validate(&mut source)?;
        }
        if kind == "docx" {
            docx_integrity::validate(&mut source)?;
        }
        self.publish(source)
    }

    #[cfg(test)]
    pub fn commit(&self, allow_empty: bool) -> Result<i64, AppError> {
        self.publish(self.source(allow_empty)?)
    }

    /// Undo only a final entry actually published by this owner, e.g. when its
    /// conversation was deleted while rendering. Never remove a collision.
    pub fn discard_publication(&self) -> Result<(), AppError> {
        if self.published.load(Ordering::Acquire) {
            match self.directory.remove_file(&self.final_name) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            #[cfg(unix)]
            self.directory.open(".")?.sync_all()?;
            self.published.store(false, Ordering::Release);
        }
        Ok(())
    }

    fn publish(&self, mut source: fs::File) -> Result<i64, AppError> {
        let size = source.metadata()?.len();
        let temporary = format!(".pending-{}", uuid::Uuid::new_v4());
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = self.directory.open_with(&temporary, &options)?;
        let result = (|| -> io::Result<()> {
            source.seek(SeekFrom::Start(0))?;
            if io::copy(&mut source.take(size), &mut output)? != size {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Staged artifact changed during publication",
                ));
            }
            output.sync_all()?;
            drop(output);
            match self.directory.symlink_metadata(&self.final_name) {
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "Artifact destination already exists",
                    ))
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            self.directory
                .rename(&temporary, &self.directory, &self.final_name)?;
            self.published.store(true, Ordering::Release);
            #[cfg(unix)]
            self.directory.open(".")?.sync_all()?;
            Ok(())
        })();
        let _ = self.directory.remove_file(&temporary);
        result?;
        i64::try_from(size).map_err(|_| AppError::internal("Artifact size exceeds supported range"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reserve_fixture_directory(parent: &Path) -> Result<(PathBuf, String), AppError> {
        let (_, id) = reserve_directory(parent, Path::new("."))?;
        Ok((parent.join(&id), id))
    }
    fn stage_fixture(final_path: &Path) -> StagedOutput {
        let directory =
            Dir::open_ambient_dir(final_path.parent().unwrap(), ambient_authority()).unwrap();
        StagedOutput::from_reserved(directory, final_path.file_name().unwrap().to_str().unwrap())
            .unwrap()
    }

    fn directory_link(link: &Path, target: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        {
            let result = std::process::Command::new("pwsh").args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:TEST_LINK -Target $env:TEST_TARGET -ErrorAction Stop | Out-Null"])
                .env("TEST_LINK", link).env("TEST_TARGET", target).output().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }

    // Windows directory handles can deny rename/delete sharing entirely. That
    // is also fail-closed containment: assert the OS denial and exercise normal
    // publication through the still-held directory rather than ignoring it.
    fn replace_parent(parent: &Path, moved: &Path, outside: &Path) -> PathBuf {
        match fs::rename(parent, moved) {
            Ok(()) => {
                directory_link(parent, outside);
                moved.to_owned()
            }
            Err(error) => {
                #[cfg(windows)]
                {
                    assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error}");
                    assert!(parent.is_dir());
                    parent.to_owned()
                }
                #[cfg(not(windows))]
                panic!("directory replacement failed: {error}");
            }
        }
    }

    #[test]
    fn discard_removes_only_its_published_entry_and_preserves_collisions() {
        let root = tempfile::tempdir().unwrap();
        let (directory, id) = reserve_directory(root.path(), Path::new("generated")).unwrap();
        let output = StagedOutput::from_reserved(directory, "file.txt").unwrap();
        let final_path = root.path().join("generated").join(id).join("file.txt");
        fs::write(&final_path, b"other owner").unwrap();
        output.discard_publication().unwrap();
        assert_eq!(fs::read(&final_path).unwrap(), b"other owner");
        fs::remove_file(&final_path).unwrap();
        fs::write(&output.path, b"published text").unwrap();
        output.commit(false).unwrap();
        output.discard_publication().unwrap();
        output.discard_publication().unwrap();
        assert!(!final_path.exists());
    }

    #[test]
    fn external_parent_cannot_receive_a_reservation() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("generated")).unwrap();
        directory_link(&root.path().join("generated/exports"), outside.path());
        assert!(reserve_directory(root.path(), Path::new("generated/exports")).is_err());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        #[cfg(windows)]
        fs::remove_dir(root.path().join("generated/exports")).unwrap();
    }

    #[test]
    fn rendering_and_publication_survive_parent_path_replacement_without_escape() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (directory, id) =
            reserve_directory(root.path(), Path::new("generated/exports")).unwrap();
        let parent = root.path().join("generated/exports");
        let moved = root.path().join("held-exports");
        let held = replace_parent(&parent, &moved, outside.path());
        let output = StagedOutput::from_reserved(directory, "report.pdf").unwrap();
        assert!(!output.path.starts_with(root.path()));
        assert!(!output.path.starts_with(outside.path()));
        let private_path = output.path.clone();
        let pdf = pdf_integrity::fixture_pdf();
        fs::write(&private_path, &pdf).unwrap();
        output.commit_for("pdf").unwrap();
        assert_eq!(fs::read(held.join(id).join("report.pdf")).unwrap(), pdf);
        drop(output);
        assert!(!private_path.exists());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        #[cfg(windows)]
        if held == moved {
            fs::remove_dir(parent).unwrap();
        }
    }

    #[test]
    fn root_replacement_cannot_redirect_an_already_reserved_artifact() {
        let container = tempfile::tempdir().unwrap();
        let root = container.path().join("root");
        fs::create_dir(&root).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (directory, id) = reserve_directory(&root, Path::new("generated/images")).unwrap();
        let output = StagedOutput::from_reserved(directory, "image.png").unwrap();
        let moved = container.path().join("held-root");
        let held = replace_parent(&root, &moved, outside.path());
        fs::write(&output.path, b"\x89PNG\r\n\x1a\nfixture").unwrap();
        output.commit_for("image").unwrap();
        assert!(held
            .join("generated/images")
            .join(id)
            .join("image.png")
            .is_file());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        #[cfg(windows)]
        if held == moved {
            fs::remove_dir(&root).unwrap();
        }
    }

    #[test]
    fn traversal_and_path_shaped_filenames_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        for path in ["../escape", "/absolute", "generated/../../escape"] {
            assert!(reserve_directory(root.path(), Path::new(path)).is_err());
        }
        let (directory, _) = reserve_directory(root.path(), Path::new("generated")).unwrap();
        for name in [
            "",
            ".",
            "..",
            "../outside",
            "nested/file",
            "..\\outside",
            "C:\\outside",
        ] {
            assert!(
                StagedOutput::from_reserved(directory.open_dir(".").unwrap(), name).is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn every_rendered_kind_keeps_its_extension_and_publishes_matching_bytes() {
        let root = tempfile::tempdir().unwrap();
        let pdf = pdf_integrity::fixture_pdf();
        let docx = docx_integrity::fixture_docx();
        for (kind, name, bytes) in [
            ("pdf", "doc.pdf", pdf.as_slice()),
            ("docx", "doc.docx", docx.as_slice()),
            ("image", "image.png", &b"\x89PNG\r\n\x1a\nfixture"[..]),
            ("audio", "audio.wav", &b"RIFF0000WAVEfixture"[..]),
            ("video", "video.webm", &b"\x1a\x45\xdf\xa3fixture"[..]),
        ] {
            let (directory, id) = reserve_directory(root.path(), Path::new("generated")).unwrap();
            let output = StagedOutput::from_reserved(directory, name).unwrap();
            assert_eq!(output.path.extension(), Path::new(name).extension());
            fs::write(&output.path, bytes).unwrap();
            assert_eq!(output.commit_for(kind).unwrap(), bytes.len() as i64);
            assert_eq!(
                fs::read(root.path().join("generated").join(id).join(name)).unwrap(),
                bytes
            );
        }
    }

    #[test]
    fn corrupt_pdf_with_correct_magic_is_not_published_and_staging_is_cleaned() {
        let root = tempfile::tempdir().unwrap();
        let (dir, id) = reserve_directory(root.path(), Path::new("generated")).unwrap();
        let output = StagedOutput::from_reserved(dir, "corrupt.pdf").unwrap();
        let private = output.path.clone();
        fs::write(&private, b"%PDF-fake\n%%EOF").unwrap();
        assert!(output.commit_for("pdf").is_err());
        assert!(!root
            .path()
            .join("generated")
            .join(id)
            .join("corrupt.pdf")
            .exists());
        drop(output);
        assert!(!private.exists());
    }

    #[test]
    fn corrupt_docx_with_correct_magic_is_not_published_and_staging_is_cleaned() {
        let root = tempfile::tempdir().unwrap();
        let (dir, id) = reserve_directory(root.path(), Path::new("generated")).unwrap();
        let output = StagedOutput::from_reserved(dir, "bad.docx").unwrap();
        let private = output.path.clone();
        fs::write(&private, b"PK\x03\x04fake").unwrap();
        assert!(output.commit_for("docx").is_err());
        assert!(!root
            .path()
            .join("generated")
            .join(id)
            .join("bad.docx")
            .exists());
        drop(output);
        assert!(!private.exists());
    }

    #[cfg(unix)]
    #[test]
    fn final_and_staging_links_are_rejected_without_touching_their_targets() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let sentinel = outside.path().join("target.pdf");
        fs::write(&sentinel, b"%PDF-preserve").unwrap();
        let (directory, id) = reserve_directory(root.path(), Path::new("generated")).unwrap();
        let output = StagedOutput::from_reserved(directory, "doc.pdf").unwrap();
        std::os::unix::fs::symlink(&sentinel, &output.path).unwrap();
        assert!(output.commit_for("pdf").is_err());
        fs::remove_file(&output.path).unwrap();
        fs::write(&output.path, pdf_integrity::fixture_pdf()).unwrap();
        std::os::unix::fs::symlink(
            &sentinel,
            root.path().join("generated").join(&id).join("doc.pdf"),
        )
        .unwrap();
        assert!(output.commit_for("pdf").is_err());
        assert_eq!(fs::read(sentinel).unwrap(), b"%PDF-preserve");
        assert_eq!(
            fs::read_dir(root.path().join("generated").join(id))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn parallel_requests_with_identical_names_have_independent_bytes_for_every_kind() {
        let root = tempfile::tempdir().unwrap();
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for extension in ["png", "webm", "wav", "pdf", "docx", "txt"] {
                for index in 0..8 {
                    let parent = root.path();
                    handles.push(scope.spawn(move || {
                        let (dir, _) = reserve_fixture_directory(parent).unwrap();
                        let final_path = dir.join(format!("same-title.{extension}"));
                        let output = stage_fixture(&final_path);
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
        let (one, _) = reserve_fixture_directory(temp.path()).unwrap();
        let (two, _) = reserve_fixture_directory(temp.path()).unwrap();
        let good = two.join("same.png");
        fs::write(&good, b"keep").unwrap();
        let final_path = one.join("same.png");
        let pending;
        {
            let output = stage_fixture(&final_path);
            pending = output.path.clone();
            fs::write(&output.path, b"").unwrap();
            assert!(output.commit(false).is_err());
            fs::write(&output.path, b"interrupted bytes").unwrap();
        }
        assert!(!pending.exists());
        assert!(!final_path.exists());
        assert_eq!(fs::read(&good).unwrap(), b"keep");
        let output = stage_fixture(&final_path);
        fs::write(&output.path, b"complete").unwrap();
        fs::write(&final_path, b"already published").unwrap();
        assert!(output.commit(false).is_err());
        assert_eq!(fs::read(&final_path).unwrap(), b"already published");
    }
    #[test]
    fn signature_mismatch_cannot_be_published_as_ready_media() {
        let temp = tempfile::tempdir().unwrap();
        let final_path = temp.path().join("image.png");
        let output = stage_fixture(&final_path);
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
