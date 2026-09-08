from pathlib import Path


def replace_exact(path: str, old: str, new: str, expected: int = 1) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    found = text.count(old)
    if found != expected:
        raise SystemExit(f"{path}: expected {expected} occurrences, found {found}: {old[:120]!r}")
    file.write_text(text.replace(old, new), encoding="utf-8")


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

# Every loop exit assigns the durable run status before it is consumed. Avoid
# a redundant initial value that strict Clippy correctly identifies as unused.
replace_exact(
    "src-tauri/src/local_agent.rs",
    '    let mut run_status = "completed";\n',
    '    let mut run_status;\n',
)

# These two internal orchestration boundaries intentionally carry the model,
# sandbox, plan/run and tool context explicitly. Keeping those security-relevant
# inputs visible is preferable to hiding them inside a loosely scoped bag.
replace_exact(
    "src-tauri/src/local_agent.rs",
    "async fn request_agent_decision(\n",
    "#[allow(clippy::too_many_arguments)]\nasync fn request_agent_decision(\n",
)
replace_exact(
    "src-tauri/src/local_agent.rs",
    "async fn execute_tool(\n",
    "#[allow(clippy::too_many_arguments)]\nasync fn execute_tool(\n",
)

print("coding workspace compile repairs applied")
