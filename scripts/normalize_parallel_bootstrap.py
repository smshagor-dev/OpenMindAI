from pathlib import Path

path = Path("scripts/integrate_coding_workspace.py")
text = path.read_text(encoding="utf-8")


def required_replace(old: str, new: str, label: str) -> None:
    global text
    if old not in text:
        raise SystemExit(f"parallel bootstrap normalization anchor missing: {label}")
    text = text.replace(old, new, 1)


# Current main already passes context_window_tokens to request_agent_decision.
required_replace(
    """    '''                &model.id,\n                &sandbox_mode,\n            ) => result?,\n''',\n    '''                &model.id,\n                &sandbox_mode,\n                &plan_text,\n            ) => result?,\n''',\n""",
    """    '''                &model.id,\n                &sandbox_mode,\n                context_window_tokens,\n            ) => result?,\n''',\n    '''                &model.id,\n                &sandbox_mode,\n                context_window_tokens,\n                &plan_text,\n            ) => result?,\n''',\n""",
    "decision call context window",
)

# Current main request_agent_decision signature also owns context_window_tokens.
required_replace(
    """    '''    sandbox_mode: &str,\n) -> Result<Value, AppError> {\n''',\n    '''    sandbox_mode: &str,\n    plan_text: &str,\n) -> Result<AgentDecisionResult, AppError> {\n''',\n""",
    """    '''    sandbox_mode: &str,\n    context_window_tokens: usize,\n) -> Result<Value, AppError> {\n''',\n    '''    sandbox_mode: &str,\n    context_window_tokens: usize,\n    plan_text: &str,\n) -> Result<AgentDecisionResult, AppError> {\n''',\n""",
    "decision signature context window",
)

# Replace the older prompt-format bootstrap with a current-main aware transform.
prompt_start_marker = "patch(\n    \"src-tauri/src/local_agent.rs\",\n    '''        \"Project: {}\\\\nStep: {}/{}\\\\nProject instructions:"
prompt_end_marker = "# Estimate prompt before JSON moves strings and collect llama usage when supplied.\n"
if prompt_start_marker not in text:
    raise SystemExit("parallel bootstrap normalization anchor missing: prompt-format block")
start = text.index(prompt_start_marker)
end = text.index(prompt_end_marker, start)
replacement = r'''local_path = Path("src-tauri/src/local_agent.rs")
local_text = local_path.read_text(encoding="utf-8")
current_prompt = '"Project: {}\\nStep: {}/{}\\nContext selection: selectedChars={}/{} compressed={}\\nProject instructions:'
planned_prompt = '"Project: {}\\nStep: {}/{}\\nActive persisted plan:\\n{}\\n\\nContext selection: selectedChars={}/{} compressed={}\\nProject instructions:'
if current_prompt not in local_text:
    raise SystemExit("current prompt-context format not found")
local_text = local_text.replace(current_prompt, planned_prompt, 1)
current_args = '''        MAX_AGENT_STEPS,
        prompt_context.selected_chars,
'''
planned_args = '''        MAX_AGENT_STEPS,
        plan_text,
        prompt_context.selected_chars,
'''
if current_args not in local_text:
    raise SystemExit("current prompt-context arguments not found")
local_text = local_text.replace(current_args, planned_args, 1)
local_path.write_text(local_text, encoding="utf-8")

'''
text = text[:start] + replacement + text[end:]

path.write_text(text, encoding="utf-8")
print("parallel coding bootstrap normalized for current main")
