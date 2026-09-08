from pathlib import Path
import re


def replace_exact(path: str, old: str, new: str, expected: int = 1) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    found = text.count(old)
    if found != expected:
        raise SystemExit(f"{path}: expected {expected} occurrences, found {found}: {old[:120]!r}")
    file.write_text(text.replace(old, new), encoding="utf-8")


def replace_regex(path: str, pattern: str, replacement: str, expected: int = 1) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    updated, count = re.subn(pattern, replacement, text, flags=re.MULTILINE)
    if count != expected:
        raise SystemExit(f"{path}: expected {expected} regex matches, found {count}: {pattern!r}")
    file.write_text(updated, encoding="utf-8")


# serde_json errors must be mapped into the app's explicit error boundary.
replace_exact(
    "src-tauri/src/coding_control.rs",
    "serde_json::to_string(&plan)?",
    "serde_json::to_string(&plan).map_err(|error| AppError::internal(error.to_string()))?",
    expected=2,
)

# Sandbox provider is optional when the host isolation provider is unavailable.
replace_exact(
    "src-tauri/src/coding_eval.rs",
    'format!("{}: {}", capability.provider, capability.message),',
    'format!(\n            "{}: {}",\n            capability.provider.as_deref().unwrap_or("unavailable"),\n            capability.message\n        ),',
)

for detail in (
    "repository intelligence labels ordinary repository/source content as untrusted data and excludes credential-like files",
    "multi-file transaction and symbol navigation modules are compiled into the coding workspace",
    "delivery operations are allowlisted, remote mutations require exact approval, and merge checks require local plus repository validation",
):
    replace_exact(
        "src-tauri/src/coding_eval.rs",
        f'        "{detail}",',
        f'        "{detail}".to_string(),',
    )

# This import is no longer needed after the repository intelligence refactor.
replace_exact(
    "src-tauri/src/coding_intelligence.rs",
    "use serde_json::Value;\n",
    "",
)

# The run loop deliberately assigns its final durable status at multiple exit
# points. The pre-format generated source makes removing the redundant seed
# brittle, so scope the rustc lint to this one orchestration boundary.
replace_regex(
    "src-tauri/src/local_agent.rs",
    r'^(?P<indent>[ \t]*)async fn run_agent_message\($',
    r'\g<indent>#[allow(unused_assignments)]\n\g<indent>async fn run_agent_message(',
)

# These two internal orchestration boundaries intentionally carry the model,
# sandbox, plan/run and tool context explicitly. Keeping those security-relevant
# inputs visible is preferable to hiding them inside a loosely scoped bag.
replace_regex(
    "src-tauri/src/local_agent.rs",
    r'^(?P<indent>[ \t]*)async fn request_agent_decision\($',
    r'\g<indent>#[allow(clippy::too_many_arguments)]\n\g<indent>async fn request_agent_decision(',
)
replace_regex(
    "src-tauri/src/local_agent.rs",
    r'^(?P<indent>[ \t]*)async fn execute_tool\($',
    r'\g<indent>#[allow(clippy::too_many_arguments)]\n\g<indent>async fn execute_tool(',
)

print("coding workspace compile repairs applied")
