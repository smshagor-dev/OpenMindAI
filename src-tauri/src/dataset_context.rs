use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{app_error::AppError, portable_root::PortableRootManager};

const MAX_DATASETS: usize = 4;
const MAX_FILES_PER_DATASET: usize = 6;
const MAX_SNIPPET_CHARS: usize = 1_200;
const MAX_CONTEXT_CHARS: usize = 5_500;

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
    root: PathBuf,
    score: i32,
    files: Vec<DatasetManifestFile>,
}

pub(crate) fn build_dataset_context(
    root: &PortableRootManager,
    prompt: &str,
    model_name: &str,
    routing_reason: &str,
) -> Result<Option<String>, AppError> {
    let base = root.resolve_relative("datasets/openmindai")?;
    if !base.is_dir() {
        return Ok(None);
    }

    let keywords = prompt_keywords(prompt);
    if keywords.is_empty() {
        return Ok(None);
    }

    let mut candidates = Vec::new();
    collect_manifests(&base, &base, &keywords, &mut candidates)?;
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
    base: &Path,
    dir: &Path,
    keywords: &[String],
    candidates: &mut Vec<DatasetCandidate>,
) -> Result<(), AppError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_manifests(base, &path, keywords, candidates)?;
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()) != Some("openmindai-dataset.json") {
            continue;
        }
        let content = fs::read_to_string(&path)?;
        let manifest: DatasetManifest = serde_json::from_str(&content).map_err(|error| {
            AppError::internal(format!("dataset manifest parse failed: {error}"))
        })?;
        let dataset_root = path.parent().unwrap_or(base).to_path_buf();
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
                root: dataset_root,
                score,
                files: manifest.files,
            });
        }
    }
    Ok(())
}

fn dataset_snippets(
    dataset_root: &Path,
    files: &[DatasetManifestFile],
    keywords: &[String],
) -> Result<Vec<String>, AppError> {
    let mut scored = files
        .iter()
        .filter(|file| supported_context_file(&file.path))
        .map(|file| (score_text(&file.path.to_ascii_lowercase(), keywords), file))
        .collect::<Vec<_>>();
    scored.sort_by_key(|item| std::cmp::Reverse(item.0));

    let mut snippets = Vec::new();
    for (_, file) in scored.into_iter().take(MAX_FILES_PER_DATASET) {
        let path = dataset_root.join(&file.path);
        if !path.is_file() {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
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
