from pathlib import Path


def replace_exact(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    if old not in text:
        raise SystemExit(f"anchor missing in {path}: {old[:100]!r}")
    file.write_text(text.replace(old, new, 1), encoding="utf-8")


# Register the prompt-context planner as a normal Rust module.
replace_exact(
    "src-tauri/src/lib.rs",
    "mod openagent_context;\nmod openagent_runs;",
    "mod openagent_context;\nmod openagent_prompt_context;\nmod openagent_runs;",
)

# Make repository selection content-aware and allow goal-centered excerpts from
# larger source files while keeping repository guidance under the original
# strict file-size bound.
replace_exact(
    "src-tauri/src/openagent_context.rs",
    "use std::{\n    collections::HashSet,\n    fs,\n    path::{Path, PathBuf},\n};",
    "use std::{\n    collections::HashSet,\n    fs,\n    io::Read,\n    path::{Path, PathBuf},\n};",
)
replace_exact(
    "src-tauri/src/openagent_context.rs",
    "const MAX_FILE_BYTES: u64 = 64 * 1024;\nconst MAX_PACK_CHARS: usize = 18_000;",
    "const MAX_FILE_BYTES: u64 = 64 * 1024;\nconst MAX_SOURCE_FILE_BYTES: u64 = 512 * 1024;\nconst MAX_RELEVANCE_SCAN_BYTES: u64 = 64 * 1024;\nconst MAX_SOURCE_EXCERPT_CHARS: usize = 4_000;\nconst EXCERPT_CONTEXT_LINES: usize = 8;\nconst MAX_PACK_CHARS: usize = 18_000;",
)
replace_exact(
    "src-tauri/src/openagent_context.rs",
    "        let relevant = relevant_candidates(&root, &files, &terms, &all_instructions);",
    "        let relevant = relevant_candidates(&root, &files, &terms, &all_instructions)?;",
)
replace_exact(
    "src-tauri/src/openagent_context.rs",
    "        for path in relevant.iter().take(MAX_RELEVANT_FILES) {\n            if let Some(content) = read_bounded_text(path)? {",
    "        for path in relevant.iter().take(MAX_RELEVANT_FILES) {\n            if let Some(content) = read_source_excerpt(path, &terms)? {",
)
old_relevant = '''fn relevant_candidates(
    root: &Path,
    files: &[PathBuf],
    terms: &HashSet<String>,
    instructions: &[PathBuf],
) -> Vec<PathBuf> {
    let instruction_set = instructions.iter().collect::<HashSet<_>>();
    let mut scored = files
        .iter()
        .filter(|path| {
            !instruction_set.contains(path)
                && !guidance_like_file(root, path)
                && safe_context_file(path)
        })
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
'''
new_relevant = '''fn relevant_candidates(
    root: &Path,
    files: &[PathBuf],
    terms: &HashSet<String>,
    instructions: &[PathBuf],
) -> Result<Vec<PathBuf>, AppError> {
    let instruction_set = instructions.iter().collect::<HashSet<_>>();
    let mut scored = Vec::new();
    for path in files.iter().filter(|path| {
        !instruction_set.contains(path)
            && !guidance_like_file(root, path)
            && safe_context_file(path)
    }) {
        let relative = relative_display(root, path);
        let normalized_path = relative.to_ascii_lowercase();
        let manifest = MANIFEST_FILES
            .iter()
            .any(|name| normalized_path.ends_with(&name.to_ascii_lowercase()));
        let path_hits = terms
            .iter()
            .filter(|term| normalized_path.contains(term.as_str()))
            .count();
        let content_hits = read_relevance_text(path)?
            .map(|content| {
                let normalized_content = content.to_ascii_lowercase();
                terms
                    .iter()
                    .filter(|term| normalized_content.contains(term.as_str()))
                    .count()
            })
            .unwrap_or(0);
        let score = usize::from(manifest) * 30 + path_hits * 50 + content_hits * 12;
        if score > 0 {
            scored.push((score, relative, path.clone()));
        }
    }
    scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    Ok(scored.into_iter().map(|(_, _, path)| path).collect())
}

fn read_relevance_text(path: &Path) -> Result<Option<String>, AppError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_SOURCE_FILE_BYTES
    {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(metadata.len().min(MAX_RELEVANCE_SCAN_BYTES) as usize);
    let mut limited = fs::File::open(path)?.take(MAX_RELEVANCE_SCAN_BYTES);
    limited.read_to_end(&mut bytes)?;
    Ok(String::from_utf8(bytes).ok())
}

fn read_source_excerpt(path: &Path, terms: &HashSet<String>) -> Result<Option<String>, AppError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_SOURCE_FILE_BYTES
    {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    let Some(content) = String::from_utf8(bytes).ok() else {
        return Ok(None);
    };
    if content.chars().count() <= MAX_SOURCE_EXCERPT_CHARS {
        return Ok(Some(content));
    }

    let lines = content.lines().collect::<Vec<_>>();
    let best = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let normalized = line.to_ascii_lowercase();
            let hits = terms
                .iter()
                .filter(|term| normalized.contains(term.as_str()))
                .count();
            (hits, index)
        })
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)));

    if let Some((hits, index)) = best.filter(|(hits, _)| *hits > 0) {
        let start = index.saturating_sub(EXCERPT_CONTEXT_LINES);
        let end = (index + EXCERPT_CONTEXT_LINES + 1).min(lines.len());
        let excerpt = lines[start..end].join("\n");
        return Ok(Some(format!(
            "[goal-centered excerpt around line {}; matchedTerms={hits}]\n{}",
            index + 1,
            truncate_chars(&excerpt, MAX_SOURCE_EXCERPT_CHARS)
        )));
    }

    Ok(Some(format!(
        "[source prefix excerpt]\n{}",
        truncate_chars(&content, MAX_SOURCE_EXCERPT_CHARS)
    )))
}
'''
replace_exact("src-tauri/src/openagent_context.rs", old_relevant, new_relevant)

insert_test_anchor = '''    #[test]
    fn context_does_not_follow_symlinked_files_or_guidance() {
'''
new_tests = '''    #[test]
    fn content_relevance_selects_source_even_when_path_is_generic() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(
            temp.path().join("src/opaque.rs"),
            "fn refresh_session_token_rotation() {}",
        )
        .unwrap();

        let context = build_repository_context(
            &[("root".to_string(), temp.path().display().to_string())],
            "repair refresh session token rotation",
        )
        .unwrap();

        assert!(context.contains("RELEVANT FILE [src/opaque.rs]"));
        assert!(context.contains("refresh_session_token_rotation"));
    }

    #[test]
    fn large_relevant_source_uses_goal_centered_excerpt() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("src/auth")).unwrap();
        let mut source = "// filler line\n".repeat(6_000);
        source.push_str("fn refresh_session_goal_marker() {}\n");
        source.push_str(&"// tail line\n".repeat(200));
        fs::write(temp.path().join("src/auth/session.rs"), source).unwrap();

        let context = build_repository_context(
            &[("root".to_string(), temp.path().display().to_string())],
            "fix auth session refresh goal marker",
        )
        .unwrap();

        assert!(context.contains("RELEVANT FILE [src/auth/session.rs]"));
        assert!(context.contains("goal-centered excerpt"));
        assert!(context.contains("refresh_session_goal_marker"));
        assert!(!context.contains(&"// filler line\n".repeat(500)));
    }

''' + insert_test_anchor
replace_exact("src-tauri/src/openagent_context.rs", insert_test_anchor, new_tests)

# Prompt planner: one global budget derived from the active model window. It
# preserves the latest tool observations and redistributes unused section
# budget rather than independently truncating every context source.
prompt_module = r'''use std::collections::VecDeque;

const CHARS_PER_TOKEN_ESTIMATE: usize = 3;
const PROMPT_MARGIN_TOKENS: usize = 256;
const MIN_OUTPUT_TOKENS: usize = 640;
const MAX_OUTPUT_TOKENS: usize = 1_536;
const FIXED_USER_OVERHEAD_CHARS: usize = 900;
const MAX_SELECTED_CONTEXT_CHARS: usize = 24_000;

pub(crate) struct PromptContextInput<'a> {
    pub project_instructions: &'a str,
    pub repository_context: &'a str,
    pub workspace_context: &'a str,
    pub conversation_context: &'a str,
    pub goal: &'a str,
    pub transcript: &'a VecDeque<String>,
    pub context_window_tokens: usize,
    pub system_chars: usize,
}

pub(crate) struct PromptContextPack {
    pub project_instructions: String,
    pub repository_context: String,
    pub workspace_context: String,
    pub conversation_context: String,
    pub goal: String,
    pub transcript: String,
    pub max_output_tokens: usize,
    pub budget_chars: usize,
    pub selected_chars: usize,
    pub compressed: bool,
}

pub(crate) fn build_prompt_context(input: PromptContextInput<'_>) -> PromptContextPack {
    let context_window_tokens = input.context_window_tokens.clamp(4_096, 32_768);
    let max_output_tokens = (context_window_tokens / 6)
        .clamp(MIN_OUTPUT_TOKENS, MAX_OUTPUT_TOKENS);
    let input_tokens = context_window_tokens
        .saturating_sub(max_output_tokens)
        .saturating_sub(PROMPT_MARGIN_TOKENS);
    let budget_chars = input_tokens
        .saturating_mul(CHARS_PER_TOKEN_ESTIMATE)
        .saturating_sub(input.system_chars)
        .saturating_sub(FIXED_USER_OVERHEAD_CHARS)
        .min(MAX_SELECTED_CONTEXT_CHARS);

    let transcript_raw = input
        .transcript
        .iter()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n\n");
    let raw = [
        input.goal,
        transcript_raw.as_str(),
        input.repository_context,
        input.project_instructions,
        input.conversation_context,
        input.workspace_context,
    ];
    let caps = [6_000usize, 8_000, 10_000, 5_000, 5_000, 5_000];
    let weights = [30usize, 25, 22, 10, 8, 5];
    let lengths = raw
        .iter()
        .zip(caps)
        .map(|(value, cap)| value.chars().count().min(cap))
        .collect::<Vec<_>>();
    let budgets = allocate_budgets(&lengths, &weights, budget_chars);

    let goal = compress_middle(input.goal, budgets[0]);
    let transcript = compress_transcript(input.transcript, budgets[1]);
    let repository_context = compress_middle(input.repository_context, budgets[2]);
    let project_instructions = compress_head(input.project_instructions, budgets[3]);
    let conversation_context = compress_tail(input.conversation_context, budgets[4]);
    let workspace_context = compress_middle(input.workspace_context, budgets[5]);
    let selected_chars = [
        goal.as_str(),
        transcript.as_str(),
        repository_context.as_str(),
        project_instructions.as_str(),
        conversation_context.as_str(),
        workspace_context.as_str(),
    ]
    .iter()
    .map(|value| value.chars().count())
    .sum::<usize>();
    let raw_chars = raw.iter().map(|value| value.chars().count()).sum::<usize>();

    PromptContextPack {
        project_instructions,
        repository_context,
        workspace_context,
        conversation_context,
        goal,
        transcript,
        max_output_tokens,
        budget_chars,
        selected_chars,
        compressed: selected_chars < raw_chars,
    }
}

fn allocate_budgets(lengths: &[usize], weights: &[usize], total: usize) -> Vec<usize> {
    debug_assert_eq!(lengths.len(), weights.len());
    if total == 0 {
        return vec![0; lengths.len()];
    }
    let weight_sum = weights.iter().sum::<usize>().max(1);
    let mut budgets = lengths
        .iter()
        .zip(weights)
        .map(|(length, weight)| (*length).min(total.saturating_mul(*weight) / weight_sum))
        .collect::<Vec<_>>();
    let mut remaining = total.saturating_sub(budgets.iter().sum::<usize>());

    // Reuse budget left by empty/small sections in priority order. Goal and
    // recent tool history intentionally win because they carry the current
    // task and the newest execution evidence.
    while remaining > 0 {
        let mut progressed = false;
        for index in 0..lengths.len() {
            if budgets[index] < lengths[index] {
                let grant = (lengths[index] - budgets[index]).min(remaining);
                budgets[index] += grant;
                remaining -= grant;
                progressed = true;
                if remaining == 0 {
                    break;
                }
            }
        }
        if !progressed {
            break;
        }
    }
    budgets
}

fn compress_transcript(transcript: &VecDeque<String>, limit: usize) -> String {
    if limit == 0 || transcript.is_empty() {
        return String::new();
    }
    let mut selected = Vec::new();
    let mut used = 0usize;
    let mut omitted = 0usize;
    for item in transcript.iter().rev() {
        let separator = usize::from(!selected.is_empty()) * 2;
        let remaining = limit.saturating_sub(used + separator);
        if remaining == 0 {
            omitted += 1;
            continue;
        }
        let bounded = compress_middle(item, remaining);
        if bounded.is_empty() {
            omitted += 1;
            continue;
        }
        used += separator + bounded.chars().count();
        selected.push(bounded);
        if used >= limit {
            omitted += transcript.len().saturating_sub(selected.len());
            break;
        }
    }
    selected.reverse();
    let body = selected.join("\n\n");
    if omitted == 0 {
        return body;
    }
    let marker = format!("[older tool history omitted: {omitted} entries]\n\n");
    if marker.chars().count() >= limit {
        return compress_tail(&body, limit);
    }
    let body_limit = limit - marker.chars().count();
    format!("{marker}{}", compress_tail(&body, body_limit))
}

fn compress_head(value: &str, limit: usize) -> String {
    let count = value.chars().count();
    if count <= limit {
        return value.to_string();
    }
    if limit == 0 {
        return String::new();
    }
    let marker = "\n[context truncated]";
    let marker_len = marker.chars().count();
    if limit <= marker_len {
        return value.chars().take(limit).collect();
    }
    format!(
        "{}{}",
        value.chars().take(limit - marker_len).collect::<String>(),
        marker
    )
}

fn compress_tail(value: &str, limit: usize) -> String {
    let count = value.chars().count();
    if count <= limit {
        return value.to_string();
    }
    if limit == 0 {
        return String::new();
    }
    let marker = "[earlier context truncated]\n";
    let marker_len = marker.chars().count();
    if limit <= marker_len {
        return value.chars().rev().take(limit).collect::<String>().chars().rev().collect();
    }
    let tail = value
        .chars()
        .rev()
        .take(limit - marker_len)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{marker}{tail}")
}

fn compress_middle(value: &str, limit: usize) -> String {
    let count = value.chars().count();
    if count <= limit {
        return value.to_string();
    }
    if limit == 0 {
        return String::new();
    }
    let marker = "\n[context middle omitted]\n";
    let marker_len = marker.chars().count();
    if limit <= marker_len + 2 {
        return value.chars().take(limit).collect();
    }
    let payload = limit - marker_len;
    let head_len = payload * 2 / 3;
    let tail_len = payload - head_len;
    let head = value.chars().take(head_len).collect::<String>();
    let tail = value
        .chars()
        .rev()
        .take(tail_len)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{head}{marker}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(
        goal: &'a str,
        repository: &'a str,
        transcript: &'a VecDeque<String>,
        window: usize,
    ) -> PromptContextInput<'a> {
        PromptContextInput {
            project_instructions: &"project instruction ".repeat(600),
            repository_context: repository,
            workspace_context: &"workspace snapshot ".repeat(600),
            conversation_context: &"old chat newest chat ".repeat(600),
            goal,
            transcript,
            context_window_tokens: window,
            system_chars: 7_000,
        }
    }

    #[test]
    fn context_pack_obeys_global_budget_and_keeps_latest_execution_evidence() {
        let mut transcript = VecDeque::new();
        for index in 0..12 {
            transcript.push_back(format!("tool observation {index}: {}", "x".repeat(900)));
        }
        transcript.push_back("LATEST_FAILURE_MARKER compiler error E0425".to_string());
        let repository = "repository context ".repeat(2_000);
        let goal = "repair the session compiler failure without losing recent evidence";
        let pack = build_prompt_context(input(goal, &repository, &transcript, 8_192));

        assert!(pack.selected_chars <= pack.budget_chars);
        assert!(pack.goal.contains("session compiler failure"));
        assert!(pack.transcript.contains("LATEST_FAILURE_MARKER"));
        assert!(pack.compressed);
        assert!(pack.max_output_tokens <= MAX_OUTPUT_TOKENS);
    }

    #[test]
    fn newest_transcript_entries_survive_small_context_windows() {
        let mut transcript = VecDeque::new();
        transcript.push_back("VERY_OLD_TOOL_RESULT".repeat(400));
        transcript.push_back("NEWEST_TOOL_RESULT must survive".to_string());
        let repository = "repo ".repeat(2_000);
        let pack = build_prompt_context(input("fix newest failure", &repository, &transcript, 4_096));

        assert!(pack.transcript.contains("NEWEST_TOOL_RESULT"));
        assert!(pack.selected_chars <= pack.budget_chars);
    }

    #[test]
    fn larger_model_window_retains_more_context() {
        let transcript = VecDeque::from(["recent tool output ".repeat(500)]);
        let repository = "repository evidence ".repeat(2_000);
        let small = build_prompt_context(input("repair repository", &repository, &transcript, 4_096));
        let large = build_prompt_context(input("repair repository", &repository, &transcript, 8_192));

        assert!(large.budget_chars > small.budget_chars);
        assert!(large.selected_chars >= small.selected_chars);
    }

    #[test]
    fn context_compression_is_unicode_safe() {
        let mut transcript = VecDeque::new();
        transcript.push_back("😀".repeat(4_000));
        let repository = "λ".repeat(8_000);
        let pack = build_prompt_context(input("fix 😀 unicode context", &repository, &transcript, 4_096));
        assert!(pack.selected_chars <= pack.budget_chars);
        assert!(pack.goal.contains('😀'));
    }
}
'''
Path("src-tauri/src/openagent_prompt_context.rs").write_text(prompt_module, encoding="utf-8")

# Route each model decision through the shared global prompt budget.
replace_exact(
    "src-tauri/src/local_agent.rs",
    "    openagent_context::build_repository_context,\n    openagent_runs::OpenAgentRunRepository,",
    "    openagent_context::build_repository_context,\n    openagent_prompt_context::{build_prompt_context, PromptContextInput},\n    openagent_runs::OpenAgentRunRepository,",
)
replace_exact(
    "src-tauri/src/local_agent.rs",
    "    let plan = ModelLaunchPlanner::plan(&model, &hardware, allocate_local_port()?);\n    let endpoint = {",
    "    let plan = ModelLaunchPlanner::plan(&model, &hardware, allocate_local_port()?);\n    let context_window_tokens = plan.config.context_size as usize;\n    let endpoint = {",
)
replace_exact(
    "src-tauri/src/local_agent.rs",
    "                &model.id,\n                &sandbox_mode,\n            ) => result?,",
    "                &model.id,\n                &sandbox_mode,\n                context_window_tokens,\n            ) => result?,",
)
replace_exact(
    "src-tauri/src/local_agent.rs",
    "    model_id: &str,\n    sandbox_mode: &str,\n) -> Result<Value, AppError> {",
    "    model_id: &str,\n    sandbox_mode: &str,\n    context_window_tokens: usize,\n) -> Result<Value, AppError> {",
)
old_prompt = '''    let instructions = bounded(context.project.instructions.trim(), 5_000);
    let workspace = bounded(&context.workspace_context, 6_000);
    let repository_context = bounded(&context.repository_context, 18_000);
    let history = bounded(
        &transcript.iter().cloned().collect::<Vec<_>>().join("\n\n"),
        MAX_TRANSCRIPT_CHARS,
    );
    let user = format!(
        "Project: {}\nStep: {}/{}\nProject instructions:\n{}\n\nDiscovered repository context:\n{}\n\nWorkspace snapshot:\n{}\n\nRecent project chat:\n{}\n\nUser goal:\n{}\n\nRecent tool history:\n{}\n\nReturn the next single JSON action.",
        context.project.name,
        step + 1,
        MAX_AGENT_STEPS,
        if instructions.is_empty() { "(none)" } else { &instructions },
        if repository_context.is_empty() { "(none)" } else { &repository_context },
        workspace,
        bounded(&context.conversation_context, 7_000),
        bounded(goal, 6_000),
        if history.is_empty() { "(none)" } else { &history },
    );
'''
new_prompt = '''    let prompt_context = build_prompt_context(PromptContextInput {
        project_instructions: context.project.instructions.trim(),
        repository_context: &context.repository_context,
        workspace_context: &context.workspace_context,
        conversation_context: &context.conversation_context,
        goal,
        transcript,
        context_window_tokens,
        system_chars: system.chars().count(),
    });
    let user = format!(
        "Project: {}\nStep: {}/{}\nContext selection: selectedChars={}/{} compressed={}\nProject instructions:\n{}\n\nDiscovered repository context:\n{}\n\nWorkspace snapshot:\n{}\n\nRecent project chat:\n{}\n\nUser goal:\n{}\n\nRecent tool history:\n{}\n\nReturn the next single JSON action.",
        context.project.name,
        step + 1,
        MAX_AGENT_STEPS,
        prompt_context.selected_chars,
        prompt_context.budget_chars,
        prompt_context.compressed,
        if prompt_context.project_instructions.is_empty() {
            "(none)"
        } else {
            prompt_context.project_instructions.as_str()
        },
        if prompt_context.repository_context.is_empty() {
            "(none)"
        } else {
            prompt_context.repository_context.as_str()
        },
        if prompt_context.workspace_context.is_empty() {
            "(none)"
        } else {
            prompt_context.workspace_context.as_str()
        },
        if prompt_context.conversation_context.is_empty() {
            "(none)"
        } else {
            prompt_context.conversation_context.as_str()
        },
        if prompt_context.goal.is_empty() {
            "(none)"
        } else {
            prompt_context.goal.as_str()
        },
        if prompt_context.transcript.is_empty() {
            "(none)"
        } else {
            prompt_context.transcript.as_str()
        },
    );
'''
replace_exact("src-tauri/src/local_agent.rs", old_prompt, new_prompt)
replace_exact(
    "src-tauri/src/local_agent.rs",
    '        "max_tokens": 4096,',
    '        "max_tokens": prompt_context.max_output_tokens,',
)

print("context selection + compression source patch applied")
