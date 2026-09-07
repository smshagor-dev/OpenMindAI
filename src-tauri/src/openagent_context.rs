use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use crate::app_error::AppError;

const MAX_SCANNED_FILES: usize = 2_000;
const MAX_INSTRUCTION_FILES: usize = 12;
const MAX_RELEVANT_FILES: usize = 6;
const MAX_FILE_BYTES: u64 = 64 * 1024;
const MAX_PACK_CHARS: usize = 18_000;

const INSTRUCTION_FILES: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    "CONTRIBUTING.md",
    ".github/copilot-instructions.md",
];

const MANIFEST_FILES: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "composer.json",
    "requirements.txt",
    "Makefile",
];

pub fn build_repository_context(
    roots: &[(String, String)],
    goal: &str,
) -> Result<String, AppError> {
    let terms = goal_terms(goal);
    let mut sections = Vec::new();
    for (root_id, root_path) in roots {
        let root = PathBuf::from(root_path);
        if !root.is_dir() {
            continue;
        }
        let files = collect_files(&root)?;
        let instructions = instruction_candidates(&root, &files);
        let relevant = relevant_candidates(&root, &files, &terms, &instructions);
        let mut root_sections = Vec::new();

        for path in instructions.into_iter().take(MAX_INSTRUCTION_FILES) {
            if let Some(content) = read_bounded_text(&path)? {
                root_sections.push(format!(
                    "REPOSITORY GUIDANCE [{}]\n{}",
                    relative_display(&root, &path),
                    content
                ));
            }
        }
        for path in relevant.into_iter().take(MAX_RELEVANT_FILES) {
            if let Some(content) = read_bounded_text(&path)? {
                root_sections.push(format!(
                    "RELEVANT FILE [{}]\n{}",
                    relative_display(&root, &path),
                    content
                ));
            }
        }
        if !root_sections.is_empty() {
            sections.push(format!(
                "ROOT {root_id} ({})\n{}",
                root.display(),
                root_sections.join("\n\n")
            ));
        }
    }
    Ok(truncate_chars(&sections.join("\n\n"), MAX_PACK_CHARS))
}

fn collect_files(root: &Path) -> Result<Vec<PathBuf>, AppError> {
    let mut files = Vec::new();
    let mut directories = vec![(root.to_path_buf(), 0usize)];
    while let Some((directory, depth)) = directories.pop() {
        if depth > 8 || files.len() >= MAX_SCANNED_FILES {
            continue;
        }
        let mut entries = fs::read_dir(&directory)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if files.len() >= MAX_SCANNED_FILES {
                break;
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                if !ignored_directory(&path) {
                    directories.push((path, depth + 1));
                }
            } else if metadata.is_file() {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn instruction_candidates(root: &Path, files: &[PathBuf]) -> Vec<PathBuf> {
    let mut candidates = files
        .iter()
        .filter(|path| {
            let relative = relative_display(root, path).replace('\\', "/");
            INSTRUCTION_FILES.iter().any(|name| {
                relative == *name
                    || relative.ends_with(&format!("/{name}"))
                    || (name.starts_with(".github/") && relative == *name)
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_by_key(|path| path.components().count());
    candidates
}

fn relevant_candidates(
    root: &Path,
    files: &[PathBuf],
    terms: &HashSet<String>,
    instructions: &[PathBuf],
) -> Vec<PathBuf> {
    let instruction_set = instructions.iter().collect::<HashSet<_>>();
    let mut scored = files
        .iter()
        .filter(|path| !instruction_set.contains(path) && safe_context_file(path))
        .filter_map(|path| {
            let relative = relative_display(root, path);
            let normalized = relative.to_ascii_lowercase();
            let manifest = MANIFEST_FILES
                .iter()
                .any(|name| normalized.ends_with(&name.to_ascii_lowercase()));
            let matches = terms
                .iter()
                .filter(|term| normalized.contains(term.as_str()))
                .count();
            let score = usize::from(manifest) * 20 + matches * 10;
            (score > 0).then_some((score, relative, path.clone()))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    scored.into_iter().map(|(_, _, path)| path).collect()
}

fn read_bounded_text(path: &Path) -> Result<Option<String>, AppError> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_FILE_BYTES {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    Ok(String::from_utf8(bytes).ok())
}

fn safe_context_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if name.starts_with(".env")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.contains("credential")
        || name.contains("secret")
    {
        return false;
    }
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default(),
        "rs" | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "py"
            | "go"
            | "php"
            | "java"
            | "kt"
            | "swift"
            | "toml"
            | "json"
            | "md"
            | "yml"
            | "yaml"
    ) || MANIFEST_FILES.contains(&name)
}

fn ignored_directory(path: &Path) -> bool {
    matches!(
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default(),
        ".git" | "node_modules" | "target" | "dist" | "build" | ".next" | ".venv" | "vendor"
    )
}

fn goal_terms(goal: &str) -> HashSet<String> {
    goal.split(|character: char| {
        !character.is_alphanumeric() && character != '_' && character != '-'
    })
    .map(str::to_ascii_lowercase)
    .filter(|term| term.chars().count() >= 4)
    .filter(|term| {
        !matches!(
            term.as_str(),
            "this" | "that" | "with" | "from" | "into" | "make" | "code" | "project"
        )
    })
    .take(32)
    .collect()
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_discovers_guidance_and_relevant_source_without_secrets() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("AGENTS.md"), "Use small reviewed patches.").unwrap();
        fs::create_dir_all(temp.path().join("src/auth")).unwrap();
        fs::write(
            temp.path().join("src/auth/session.rs"),
            "fn refresh_session() {}",
        )
        .unwrap();
        fs::write(temp.path().join(".env"), "TOKEN=secret").unwrap();
        fs::write(temp.path().join("package.json"), "{}").unwrap();

        let context = build_repository_context(
            &[("root".to_string(), temp.path().display().to_string())],
            "fix auth session refresh",
        )
        .unwrap();

        assert!(context.contains("AGENTS.md"));
        assert!(context.contains("session.rs"));
        assert!(context.contains("package.json"));
        assert!(!context.contains("TOKEN=secret"));
    }

    #[test]
    fn context_does_not_follow_symlinked_files() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), "outside secret").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), temp.path().join("auth.rs")).unwrap();

        let context = build_repository_context(
            &[("root".to_string(), temp.path().display().to_string())],
            "auth",
        )
        .unwrap();
        assert!(!context.contains("outside secret"));
    }
}
