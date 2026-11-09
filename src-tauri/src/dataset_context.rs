use std::{
    collections::HashSet,
    io::{self, Read},
    path::{Path, PathBuf},
};

use cap_std::{ambient_authority, fs::Dir};
use serde::Deserialize;

use crate::{app_error::AppError, portable_root::PortableRootManager};

const MAX_DATASETS: usize = 4;
const MAX_FILES_PER_DATASET: usize = 6;
const MAX_SNIPPET_CHARS: usize = 1_200;
const MAX_CONTEXT_CHARS: usize = 5_500;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_TEXT_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_MANIFEST_FILES: usize = 4_096;
const MAX_SCAN_ENTRIES: usize = 8_192;
const MAX_SCAN_DEPTH: usize = 16;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DatasetManifest {
    dataset_id: String,
    files: Vec<DatasetManifestFile>,
}

#[derive(Debug, Deserialize)]
struct DatasetManifestFile {
    path: String,
    size: Option<u64>,
}

struct DatasetCandidate {
    id: String,
    root: Dir,
    score: i32,
    files: Vec<DatasetManifestFile>,
}

struct ManifestScan {
    visited: HashSet<PathBuf>,
    remaining_entries: usize,
}

pub(crate) fn build_dataset_context(
    root: &PortableRootManager,
    prompt: &str,
    model_name: &str,
    routing_reason: &str,
) -> Result<Option<String>, AppError> {
    let keywords = prompt_keywords(prompt);
    if keywords.is_empty() {
        return Ok(None);
    }

    // Ambient authority is used only for the user-selected portable root.
    // Every subsequent lookup stays relative to an already-open directory.
    let portable = Dir::open_ambient_dir(root.root(), ambient_authority())?;
    let base = match open_dataset_base(&portable) {
        Ok(base) => base,
        Err(error) => {
            tracing::debug!(%error, "dataset directory is unavailable or unsafe");
            return Ok(None);
        }
    };
    let mut candidates = Vec::new();
    let mut scan = ManifestScan {
        visited: HashSet::new(),
        remaining_entries: MAX_SCAN_ENTRIES,
    };
    collect_manifests(
        &base,
        Path::new("."),
        &keywords,
        &mut candidates,
        &mut scan,
        0,
    )?;
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.id.cmp(&right.id))
    });
    candidates.truncate(MAX_DATASETS);

    if candidates.is_empty() {
        return Ok(None);
    }

    let mut output = String::new();
    output.push_str("[open-mind-ai-dataset-context]\n");
    output.push_str("OpenMindAI selected local datasets relevant to the user's prompt. Use them as supporting evidence when useful, prefer the user's latest request when they conflict, and say when dataset evidence is only partial.\n");
    output.push_str(&format!(
        "Selected model: {model_name}\nModel routing feedback: {routing_reason}\n"
    ));

    for candidate in candidates {
        if output.chars().count() >= MAX_CONTEXT_CHARS {
            break;
        }
        output.push_str(&format!(
            "\nDataset: {}\nRelevance score: {}\n",
            display_dataset_id(&candidate.id),
            candidate.score
        ));
        let snippets = dataset_snippets(&candidate.root, &candidate.files, &keywords)?;
        if snippets.is_empty() {
            output.push_str("Available files:\n");
            for file in candidate.files.iter().take(MAX_FILES_PER_DATASET) {
                output.push_str(&format!("- {} ({})\n", file.path, size_label(file.size)));
            }
        } else {
            output.push_str("Relevant local evidence:\n");
            for snippet in snippets {
                output.push_str(&snippet);
                output.push('\n');
            }
        }
    }

    output.push_str("\nAnswer requirement: combine the routed model's reasoning with the selected OpenMindAI dataset evidence. If the dataset is useful, mention the dataset category/name naturally. If no selected dataset directly answers the request, provide the best answer and a short feedback note about what dataset would improve it.\n");
    Ok(Some(limit_chars(&output, MAX_CONTEXT_CHARS)))
}

fn collect_manifests(
    base: &Dir,
    dir: &Path,
    keywords: &[String],
    candidates: &mut Vec<DatasetCandidate>,
    scan: &mut ManifestScan,
    depth: usize,
) -> Result<(), AppError> {
    if depth > MAX_SCAN_DEPTH || scan.remaining_entries == 0 {
        return Ok(());
    }
    // cap-std canonicalization is relative to `base` and rejects escapes.
    let Ok(canonical) = base.canonicalize(dir) else {
        return Ok(());
    };
    if !scan.visited.insert(canonical) {
        return Ok(());
    }
    let Ok(directory) = base.open_dir(dir) else {
        return Ok(());
    };
    let Ok(entries) = directory.entries() else {
        return Ok(());
    };
    for entry in entries {
        if scan.remaining_entries == 0 {
            tracing::warn!("dataset discovery entry limit reached");
            break;
        }
        scan.remaining_entries -= 1;
        let Ok(entry) = entry else { continue };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        // Do not follow directory links (including cycles) or linked manifests.
        if kind.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        if kind.is_dir() {
            collect_manifests(base, &dir.join(name), keywords, candidates, scan, depth + 1)?;
            continue;
        }
        if !kind.is_file() || name != "openmindai-dataset.json" {
            continue;
        }
        let Ok(content) = read_bounded_text(&directory, Path::new(&name), MAX_MANIFEST_BYTES)
        else {
            tracing::warn!("skipping unreadable, unsafe or oversized dataset manifest");
            continue;
        };
        let Ok(mut manifest) = serde_json::from_str::<DatasetManifest>(&content) else {
            tracing::warn!("skipping malformed dataset manifest");
            continue;
        };
        // Validate on use: a downloaded or copied manifest is untrusted input.
        // Invalid paths must not enter retrieval scoring or the fallback listing.
        manifest.files.retain(|file| safe_manifest_path(&file.path));
        manifest.files.truncate(MAX_MANIFEST_FILES);
        let haystack = format!(
            "{} {}",
            manifest.dataset_id,
            manifest
                .files
                .iter()
                .take(40)
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        )
        .to_ascii_lowercase();
        let score = score_text(&haystack, keywords);
        if score > 0 {
            candidates.push(DatasetCandidate {
                id: manifest.dataset_id,
                root: directory.try_clone()?,
                score,
                files: manifest.files,
            });
            // Retain only the best candidates, keeping directory handles bounded.
            candidates.sort_by(|left, right| {
                right
                    .score
                    .cmp(&left.score)
                    .then_with(|| left.id.cmp(&right.id))
            });
            candidates.truncate(MAX_DATASETS);
        }
    }
    Ok(())
}

fn dataset_snippets(
    dataset_root: &Dir,
    files: &[DatasetManifestFile],
    keywords: &[String],
) -> Result<Vec<String>, AppError> {
    let mut scored = files
        .iter()
        .filter(|file| safe_manifest_path(&file.path) && supported_context_file(&file.path))
        .map(|file| (score_text(&file.path.to_ascii_lowercase(), keywords), file))
        .collect::<Vec<_>>();
    scored.sort_by_key(|item| std::cmp::Reverse(item.0));

    let mut snippets = Vec::new();
    for (_, file) in scored.into_iter().take(MAX_FILES_PER_DATASET) {
        let Ok(content) =
            read_bounded_text(dataset_root, Path::new(&file.path), MAX_TEXT_FILE_BYTES)
        else {
            continue;
        };
        let selected = best_text_window(&content, keywords);
        if selected.trim().is_empty() {
            continue;
        }
        snippets.push(format!(
            "- {} ({}): {}",
            file.path,
            size_label(file.size),
            selected.replace('\n', " ")
        ));
    }
    Ok(snippets)
}

fn open_dataset_base(portable: &Dir) -> io::Result<Dir> {
    let mut directory = portable.try_clone()?;
    for name in ["datasets", "openmindai"] {
        if !directory.symlink_metadata(name)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "linked dataset root",
            ));
        }
        directory = directory.open_dir(name)?;
    }
    Ok(directory)
}

fn safe_manifest_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4_096
        && !path
            .chars()
            .any(|ch| ch.is_control() || "\\:<>\"|?*".contains(ch))
        && path.split('/').all(|part| {
            !part.is_empty() && part != "." && part != ".." && !part.ends_with(['.', ' '])
        })
}

fn read_bounded_text(directory: &Dir, path: &Path, limit: u64) -> io::Result<String> {
    // This check rejects stable links and special files before opening. cap-std
    // also confines the actual open to this directory if a path changes later.
    if !directory.symlink_metadata(path)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular dataset file",
        ));
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        // A concurrent replacement with a FIFO must not hang the chat thread.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = directory.open_with(path, &options)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "dataset file exceeds read limit",
        ));
    }
    read_limited_utf8(file, limit)
}

fn read_limited_utf8(reader: impl Read, limit: u64) -> io::Result<String> {
    let mut bytes = Vec::new();
    // Limit the read itself, even if the file grows after metadata was checked.
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "dataset file exceeds read limit",
        ));
    }
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn supported_context_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    matches!(
        Path::new(&lower).extension().and_then(|ext| ext.to_str()),
        Some("md" | "txt" | "json" | "jsonl" | "csv" | "tsv" | "yaml" | "yml")
    )
}

fn best_text_window(content: &str, keywords: &[String]) -> String {
    let lower = content.to_ascii_lowercase();
    let index = keywords
        .iter()
        .filter_map(|keyword| lower.find(keyword))
        .min()
        .unwrap_or(0);
    let start = content[..index]
        .char_indices()
        .rev()
        .nth(250)
        .map(|(idx, _)| idx)
        .unwrap_or(0);
    let end = content[index..]
        .char_indices()
        .nth(900)
        .map(|(idx, _)| index + idx)
        .unwrap_or(content.len());
    limit_chars(content[start..end].trim(), MAX_SNIPPET_CHARS)
}

fn prompt_keywords(prompt: &str) -> Vec<String> {
    let mut words = prompt
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(|word| word.trim().to_ascii_lowercase())
        .filter(|word| word.len() >= 4 && !STOP_WORDS.contains(&word.as_str()))
        .collect::<Vec<_>>();
    words.sort();
    words.dedup();
    words.truncate(24);
    words
}

fn score_text(text: &str, keywords: &[String]) -> i32 {
    keywords
        .iter()
        .map(|keyword| if text.contains(keyword) { 4 } else { 0 })
        .sum()
}

fn display_dataset_id(id: &str) -> String {
    id.replace("HuggingFace", "OpenMindAI")
        .replace("huggingface", "openmindai")
}

fn size_label(size: Option<u64>) -> String {
    let Some(size) = size else {
        return "unknown size".to_string();
    };
    if size >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", size as f64 / 1024.0 / 1024.0 / 1024.0)
    } else if size >= 1024 * 1024 {
        format!("{:.1} MB", size as f64 / 1024.0 / 1024.0)
    } else if size >= 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{size} B")
    }
}

fn limit_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut truncated = text
        .chars()
        .take(max.saturating_sub(24))
        .collect::<String>();
    truncated.push_str("\n[dataset context truncated]");
    truncated
}

const STOP_WORDS: &[&str] = &[
    "about", "after", "also", "answer", "best", "data", "dataset", "from", "have", "into", "make",
    "model", "need", "please", "that", "their", "there", "this", "user", "with", "your",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, PortableRootManager, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let portable = temp.path().join("portable");
        let dataset = portable.join("datasets/openmindai/alpha/snapshot/main");
        fs::create_dir_all(&dataset).unwrap();
        (temp, PortableRootManager::from_root(portable), dataset)
    }

    fn manifest(directory: &Path, paths: &[String]) {
        let files = paths
            .iter()
            .map(|path| serde_json::json!({"path": path, "size": 1}))
            .collect::<Vec<_>>();
        fs::write(
            directory.join("openmindai-dataset.json"),
            serde_json::json!({"datasetId": "alpha", "files": files}).to_string(),
        )
        .unwrap();
    }

    fn context(root: &PortableRootManager) -> Option<String> {
        build_dataset_context(root, "alpha evidence", "test model", "test routing").unwrap()
    }

    #[cfg(unix)]
    fn link(target: &Path, path: &Path, _directory: bool) -> bool {
        std::os::unix::fs::symlink(target, path).unwrap();
        true
    }

    #[cfg(windows)]
    fn link(target: &Path, path: &Path, directory: bool) -> bool {
        let result = if directory {
            std::os::windows::fs::symlink_dir(target, path)
        } else {
            std::os::windows::fs::symlink_file(target, path)
        };
        match result {
            Ok(()) => true,
            Err(error) if error.raw_os_error() == Some(1314) => {
                eprintln!("symlink fixture unavailable: Windows Developer Mode/privilege required");
                false
            }
            Err(error) => panic!("symlink fixture failed: {error}"),
        }
    }

    #[test]
    fn normal_nested_evidence_is_retained() {
        let (_temp, root, dataset) = fixture();
        fs::create_dir(dataset.join("nested")).unwrap();
        fs::write(
            dataset.join("nested/alpha.txt"),
            "alpha evidence INSIDE_MARKER",
        )
        .unwrap();
        manifest(&dataset, &["nested/alpha.txt".into()]);
        assert!(context(&root).unwrap().contains("INSIDE_MARKER"));
    }

    #[test]
    fn manifest_cannot_read_parent_absolute_or_sibling_dataset_files() {
        let (temp, root, dataset) = fixture();
        let outside = temp.path().join("outside-alpha.txt");
        fs::write(&outside, "alpha OUTSIDE_MARKER").unwrap();
        fs::write(
            dataset.parent().unwrap().join("outside-alpha.txt"),
            "alpha OUTSIDE_MARKER",
        )
        .unwrap();
        let sibling = root.root().join("datasets/openmindai/other");
        fs::create_dir(&sibling).unwrap();
        fs::write(sibling.join("alpha.txt"), "alpha OUTSIDE_MARKER").unwrap();
        fs::write(dataset.join("alpha.txt"), "alpha INSIDE_MARKER").unwrap();
        manifest(
            &dataset,
            &[
                "../outside-alpha.txt".into(),
                outside.to_string_lossy().into_owned(),
                "nested/../../outside-alpha.txt".into(),
                "../../../other/alpha.txt".into(),
                "..\\outside-alpha.txt".into(),
                "C:/outside-alpha.txt".into(),
                "alpha.txt".into(),
            ],
        );
        let output = context(&root).unwrap();
        assert!(!output.contains("OUTSIDE_MARKER"));
        assert!(!output.contains("../outside-alpha.txt"));
        assert!(output.contains("INSIDE_MARKER"));
    }

    #[test]
    fn linked_text_and_intermediate_directory_cannot_supply_external_evidence() {
        let (temp, root, dataset) = fixture();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("alpha.txt"), "alpha OUTSIDE_MARKER").unwrap();
        if !link(
            &outside.join("alpha.txt"),
            &dataset.join("alpha-link.txt"),
            false,
        ) || !link(&outside, &dataset.join("linked"), true)
        {
            return;
        }
        manifest(
            &dataset,
            &["alpha-link.txt".into(), "linked/alpha.txt".into()],
        );
        assert!(!context(&root).unwrap().contains("OUTSIDE_MARKER"));
    }

    #[test]
    fn intermediate_link_within_the_same_dataset_remains_usable() {
        let (_temp, root, dataset) = fixture();
        fs::create_dir(dataset.join("nested")).unwrap();
        fs::write(dataset.join("nested/alpha.txt"), "alpha INSIDE_MARKER").unwrap();
        if !link(Path::new("nested"), &dataset.join("linked"), true) {
            return;
        }
        manifest(&dataset, &["linked/alpha.txt".into()]);
        assert!(context(&root).unwrap().contains("INSIDE_MARKER"));
    }

    #[test]
    fn linked_manifest_is_not_loaded() {
        let (temp, root, dataset) = fixture();
        let external = temp.path().join("external");
        fs::create_dir(&external).unwrap();
        let outside = external.join("alpha.txt");
        fs::write(&outside, "alpha OUTSIDE_MARKER").unwrap();
        manifest(&external, &[outside.to_string_lossy().into_owned()]);
        if !link(
            &external.join("openmindai-dataset.json"),
            &dataset.join("openmindai-dataset.json"),
            false,
        ) {
            return;
        }
        assert!(context(&root).is_none());
    }

    #[test]
    fn discovery_skips_linked_directories_and_cycles() {
        let (temp, root, dataset) = fixture();
        let external = temp.path().join("external");
        fs::create_dir(&external).unwrap();
        fs::write(external.join("alpha.txt"), "alpha OUTSIDE_MARKER").unwrap();
        manifest(&external, &["alpha.txt".into()]);
        let base = root.root().join("datasets/openmindai");
        if !link(&external, &base.join("linked"), true) || !link(&base, &dataset.join("loop"), true)
        {
            return;
        }
        assert!(context(&root).is_none());
    }

    #[test]
    fn linked_dataset_base_is_not_scanned() {
        let temp = tempfile::tempdir().unwrap();
        let portable = temp.path().join("portable");
        let external = temp.path().join("external");
        fs::create_dir_all(&portable).unwrap();
        fs::create_dir_all(external.join("openmindai")).unwrap();
        fs::write(
            external.join("openmindai/alpha.txt"),
            "alpha OUTSIDE_MARKER",
        )
        .unwrap();
        manifest(&external.join("openmindai"), &["alpha.txt".into()]);
        if !link(&external, &portable.join("datasets"), true) {
            return;
        }
        assert!(context(&PortableRootManager::from_root(portable)).is_none());
    }

    #[test]
    fn malformed_and_oversized_manifests_do_not_block_valid_datasets() {
        let (_temp, root, dataset) = fixture();
        let base = root.root().join("datasets/openmindai");
        for (name, content) in [
            ("malformed", "not json"),
            ("invalid-schema", "{\"files\": false}"),
        ] {
            let directory = base.join(name);
            fs::create_dir(&directory).unwrap();
            fs::write(directory.join("openmindai-dataset.json"), content).unwrap();
        }
        let huge = base.join("oversized");
        fs::create_dir(&huge).unwrap();
        fs::File::create(huge.join("openmindai-dataset.json"))
            .unwrap()
            .set_len(1024 * 1024 + 1)
            .unwrap();
        fs::write(dataset.join("alpha.txt"), "alpha INSIDE_MARKER").unwrap();
        manifest(&dataset, &["alpha.txt".into()]);
        assert!(context(&root).unwrap().contains("INSIDE_MARKER"));
    }

    #[test]
    fn oversized_and_invalid_utf8_text_are_skipped() {
        let (_temp, root, dataset) = fixture();
        let huge = fs::File::create(dataset.join("alpha-huge.txt")).unwrap();
        huge.set_len(4 * 1024 * 1024 + 1).unwrap();
        fs::write(dataset.join("alpha-invalid.txt"), [0xff, 0xfe]).unwrap();
        fs::write(dataset.join("alpha.txt"), "alpha INSIDE_MARKER").unwrap();
        manifest(
            &dataset,
            &[
                "alpha-huge.txt".into(),
                "alpha-invalid.txt".into(),
                "alpha.txt".into(),
            ],
        );
        let output = context(&root).unwrap();
        assert!(output.contains("INSIDE_MARKER"));
        assert!(!output.contains('\0'));
    }

    #[cfg(windows)]
    #[test]
    fn windows_junction_cannot_supply_external_evidence() {
        let (temp, root, dataset) = fixture();
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("alpha.txt"), "alpha OUTSIDE_MARKER").unwrap();
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(dataset.join("junction"))
            .arg(&outside)
            .status()
            .unwrap();
        assert!(status.success());
        manifest(&dataset, &["junction/alpha.txt".into()]);
        assert!(!context(&root).unwrap().contains("OUTSIDE_MARKER"));
    }
}

#[cfg(test)]
mod safety_tests {
    use super::*;

    #[test]
    fn portable_manifest_paths_reject_aliases_and_traversal() {
        for path in [
            "",
            ".",
            "..",
            "../alpha.txt",
            "nested/../../alpha.txt",
            "/alpha.txt",
            "C:/alpha.txt",
            "C:alpha.txt",
            "\\\\host\\alpha.txt",
            "..\\alpha.txt",
            "alpha.txt:stream",
            "nested//alpha.txt",
            "./alpha.txt",
            "nested/./alpha.txt",
            "nested/",
            "nested./alpha.txt",
            "nested /alpha.txt",
            "alpha\0.txt",
            "alpha\n.txt",
        ] {
            assert!(!safe_manifest_path(path), "{path:?}");
        }
        for path in [
            "alpha.txt",
            "nested/alpha data.csv",
            "nested/.hidden.txt",
            "বাংলা/данные.jsonl",
        ] {
            assert!(safe_manifest_path(path), "{path}");
        }
    }

    #[test]
    fn read_limit_applies_to_the_reader_and_utf8_bytes() {
        assert_eq!(read_limited_utf8("বাংলা".as_bytes(), 64).unwrap(), "বাংলা");
        assert!(read_limited_utf8(io::repeat(b'x'), 64).is_err());
        assert_eq!(read_limited_utf8(&b"alpha"[..], 5).unwrap(), "alpha");
        assert!(read_limited_utf8(&b"alpha!"[..], 5).is_err());
        assert!(read_limited_utf8(&[0xff][..], 5).is_err());
    }

    #[test]
    fn discovery_budget_depth_and_visited_set_are_enforced() {
        let temp = tempfile::tempdir().unwrap();
        let base = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();
        let keywords = vec!["alpha".into()];
        let mut candidates = Vec::new();
        let mut scan = ManifestScan {
            visited: HashSet::new(),
            remaining_entries: 0,
        };
        collect_manifests(
            &base,
            Path::new("."),
            &keywords,
            &mut candidates,
            &mut scan,
            0,
        )
        .unwrap();
        assert!(scan.visited.is_empty());
        scan.remaining_entries = MAX_SCAN_ENTRIES;
        collect_manifests(
            &base,
            Path::new("."),
            &keywords,
            &mut candidates,
            &mut scan,
            MAX_SCAN_DEPTH + 1,
        )
        .unwrap();
        assert!(scan.visited.is_empty());
        for name in ["one", "two", "three"] {
            base.create_dir(name).unwrap();
        }
        scan.remaining_entries = 1;
        collect_manifests(
            &base,
            Path::new("."),
            &keywords,
            &mut candidates,
            &mut scan,
            0,
        )
        .unwrap();
        assert_eq!(scan.remaining_entries, 0);
        let count = scan.visited.len();
        scan.remaining_entries = MAX_SCAN_ENTRIES;
        collect_manifests(
            &base,
            Path::new("./"),
            &keywords,
            &mut candidates,
            &mut scan,
            0,
        )
        .unwrap();
        assert_eq!(scan.visited.len(), count);
        assert_eq!(scan.remaining_entries, MAX_SCAN_ENTRIES);
    }

    #[cfg(unix)]
    #[test]
    fn discovered_dataset_handle_survives_path_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let dataset = temp.path().join("dataset");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&dataset).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(dataset.join("alpha.txt"), "alpha INSIDE_MARKER").unwrap();
        std::fs::write(outside.join("alpha.txt"), "alpha OUTSIDE_MARKER").unwrap();
        let handle = Dir::open_ambient_dir(&dataset, ambient_authority()).unwrap();
        std::fs::rename(&dataset, temp.path().join("moved-dataset")).unwrap();
        std::os::unix::fs::symlink(&outside, &dataset).unwrap();
        let files = vec![DatasetManifestFile {
            path: "alpha.txt".into(),
            size: Some(1),
        }];
        let snippets = dataset_snippets(&handle, &files, &["alpha".into()])
            .unwrap()
            .join("\n");
        assert!(snippets.contains("INSIDE_MARKER"));
        assert!(!snippets.contains("OUTSIDE_MARKER"));
    }
}
