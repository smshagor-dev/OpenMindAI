use std::collections::VecDeque;

const CHARS_PER_TOKEN_ESTIMATE: usize = 3;
const PROMPT_MARGIN_TOKENS: usize = 256;
const MIN_OUTPUT_TOKENS: usize = 640;
const MAX_OUTPUT_TOKENS: usize = 1_536;
const FIXED_USER_OVERHEAD_CHARS: usize = 900;
// The legacy 24K-character hard ceiling discarded evidence even in 64K/128K
// llama-server slots. Keep section priorities, but scale caps with the actual
// per-request slot size instead of an unrelated fixed global maximum.
const BASE_SECTION_BUDGET_CHARS: usize = 24_000;
const BASE_SECTION_CAPS: [usize; 6] = [6_000, 8_000, 10_000, 5_000, 5_000, 5_000];
const MAX_MODEL_CONTEXT_TOKENS: usize = 131_072;

pub(crate) struct PromptContextInput<'a> {
    pub project_instructions: &'a str,
    pub repository_context: &'a str,
    pub workspace_context: &'a str,
    pub conversation_context: &'a str,
    pub goal: &'a str,
    pub transcript: &'a VecDeque<String>,
    /// Actual llama-server tokens available in this request's slot, not
    /// configured total model context shared by multiple parallel slots.
    pub context_window_tokens: usize,
    pub system_chars: usize,
    /// Variable user-message text outside the selected context (e.g. plan).
    pub additional_user_chars: usize,
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
    // Never round a smaller real slot up: the caller already resolved the
    // runtime's /props n_ctx or divided configured context across workers.
    let context_window_tokens = input.context_window_tokens.min(MAX_MODEL_CONTEXT_TOKENS);
    let max_output_tokens = (context_window_tokens / 6).clamp(MIN_OUTPUT_TOKENS, MAX_OUTPUT_TOKENS);
    let input_tokens = context_window_tokens
        .saturating_sub(max_output_tokens)
        .saturating_sub(PROMPT_MARGIN_TOKENS);
    let budget_chars = input_tokens
        .saturating_mul(CHARS_PER_TOKEN_ESTIMATE)
        .saturating_sub(input.system_chars)
        .saturating_sub(input.additional_user_chars)
        .saturating_sub(FIXED_USER_OVERHEAD_CHARS);

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
    // Preserve the per-section priorities of the original small-window
    // prompt, while allowing larger real slots to include more tool evidence,
    // instructions, and repository context. No section can exceed the global
    // available input budget, even if its scaled cap is larger.
    let scale =
        budget_chars.saturating_add(BASE_SECTION_BUDGET_CHARS - 1) / BASE_SECTION_BUDGET_CHARS;
    let caps = BASE_SECTION_CAPS.map(|cap| cap.saturating_mul(scale.max(1)));
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
        return value
            .chars()
            .rev()
            .take(limit)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
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
            project_instructions: "project instructions",
            repository_context: repository,
            workspace_context: "workspace snapshot",
            conversation_context: "recent conversation",
            goal,
            transcript,
            context_window_tokens: window,
            system_chars: 7_000,
            additional_user_chars: 0,
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
        let pack =
            build_prompt_context(input("fix newest failure", &repository, &transcript, 4_096));

        assert!(pack.transcript.contains("NEWEST_TOOL_RESULT"));
        assert!(pack.selected_chars <= pack.budget_chars);
    }

    #[test]
    fn larger_model_window_retains_more_context() {
        let transcript = VecDeque::from(["recent tool output ".repeat(500)]);
        let repository = "repository evidence ".repeat(2_000);
        let small =
            build_prompt_context(input("repair repository", &repository, &transcript, 4_096));
        let large =
            build_prompt_context(input("repair repository", &repository, &transcript, 8_192));

        assert!(large.budget_chars > small.budget_chars);
        assert!(large.selected_chars >= small.selected_chars);
    }

    #[test]
    fn real_32k_64k_and_128k_slots_retained_context_grows() {
        let mut transcript = VecDeque::new();
        for index in 0..100 {
            transcript.push_back(format!("tool-result-{index} {}", "t".repeat(9_000)));
        }
        let repository = "repository evidence ".repeat(18_000);
        let instructions = "workspace policy ".repeat(9_000);
        let workspace = "workspace snapshot ".repeat(9_000);
        let conversation = "previous conversation ".repeat(9_000);
        let goal = "repair the failing code while retaining current user constraints".repeat(140);

        let build = |slot| {
            build_prompt_context(PromptContextInput {
                project_instructions: &instructions,
                repository_context: &repository,
                workspace_context: &workspace,
                conversation_context: &conversation,
                goal: &goal,
                transcript: &transcript,
                context_window_tokens: slot,
                system_chars: 7_000,
                additional_user_chars: 3_000,
            })
        };
        let small = build(32_768);
        let medium = build(65_536);
        let large = build(131_072);

        assert!(
            small.selected_chars > 24_000,
            "32K slot must exceed the old ceiling"
        );
        assert!(medium.budget_chars > small.budget_chars);
        assert!(large.budget_chars > medium.budget_chars);
        assert!(medium.selected_chars > small.selected_chars);
        assert!(large.selected_chars > medium.selected_chars);

        for (slot, pack) in [(32_768, small), (65_536, medium), (131_072, large)] {
            assert!(pack.selected_chars <= pack.budget_chars);
            assert!(pack.max_output_tokens <= MAX_OUTPUT_TOKENS);
            // Same conservative three-character-per-token budget used by
            // production; preserve space for both JSON output and margin.
            let prompt_chars = pack.selected_chars + 7_000 + 3_000 + FIXED_USER_OVERHEAD_CHARS;
            let input_token_upper_bound = prompt_chars.div_ceil(CHARS_PER_TOKEN_ESTIMATE);
            assert!(
                input_token_upper_bound + pack.max_output_tokens + PROMPT_MARGIN_TOKENS <= slot,
                "prompt did not fit the reported llama-server slot of {slot} tokens"
            );
        }
    }

    #[test]
    fn actual_parallel_slot_size_limits_prompt_not_configured_model_total() {
        let transcript = VecDeque::from(["latest run evidence ".repeat(25_000)]);
        let repository = "repository evidence ".repeat(14_000);
        let configured = 131_072;
        let (shared_slot, source) = crate::agent_runtime::effective_context(None, configured, 4);
        assert_eq!(source, "estimate");
        assert_eq!(shared_slot, 32_768);
        let (reported_slot, source) =
            crate::agent_runtime::effective_context(Some(65_536), configured, 4);
        assert_eq!(source, "runtime");
        assert_eq!(reported_slot, 65_536);
        let shared = build_prompt_context(input(
            "fix workspace",
            &repository,
            &transcript,
            shared_slot as usize,
        ));
        let reported = build_prompt_context(input(
            "fix workspace",
            &repository,
            &transcript,
            reported_slot as usize,
        ));
        assert!(reported.selected_chars > shared.selected_chars);
        assert!(shared.selected_chars <= shared.budget_chars);
        assert!(reported.selected_chars <= reported.budget_chars);
    }

    #[test]
    fn variable_plan_overhead_is_excluded_from_context_selection() {
        let transcript = VecDeque::from(["large tool output ".repeat(20_000)]);
        let repository = "large repository context ".repeat(16_000);
        let baseline = build_prompt_context(input("repair", &repository, &transcript, 65_536));
        let mut with_plan_input = input("repair", &repository, &transcript, 65_536);
        with_plan_input.additional_user_chars = 40_000;
        let with_plan = build_prompt_context(with_plan_input);
        assert_eq!(baseline.budget_chars - with_plan.budget_chars, 40_000);
        assert!(with_plan.selected_chars < baseline.selected_chars);
        assert!(with_plan.selected_chars <= with_plan.budget_chars);
    }

    #[test]
    fn context_compression_is_unicode_safe() {
        let mut transcript = VecDeque::new();
        transcript.push_back("😀".repeat(4_000));
        let repository = "λ".repeat(8_000);
        let pack = build_prompt_context(input(
            "fix 😀 unicode context",
            &repository,
            &transcript,
            4_096,
        ));
        assert!(pack.selected_chars <= pack.budget_chars);
        assert!(pack.goal.contains('😀'));
    }
}
