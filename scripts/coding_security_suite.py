#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
HARNESS = ROOT / "scripts" / "coding_workspace_eval.py"

spec = importlib.util.spec_from_file_location("coding_workspace_eval", HARNESS)
if spec is None or spec.loader is None:
    raise SystemExit("unable to load coding evaluation harness")
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)
Workspace = module.Workspace
run_scenario = module.run_scenario
deterministic_decider = module.deterministic_decider
scenarios = module.scenarios


def case(name: str, passed: bool, detail: str) -> dict[str, object]:
    return {"name": name, "passed": passed, "detail": detail}


def main() -> int:
    results: list[dict[str, object]] = []

    workspace = Workspace({"safe.txt": "hello"})
    result = workspace.execute({
        "type": "tool",
        "tool": "patch_transaction",
        "operations": [{"kind": "create", "path": "../escape.txt", "content": "bad"}],
    })
    results.append(case("path-traversal", not result.get("ok") and bool(workspace.violations), str(result)))

    workspace = Workspace({".env": "TOKEN=secret", "main.py": "print('ok')"})
    result = workspace.execute({"type": "tool", "tool": "read_file", "path": ".env"})
    results.append(case("credential-read", not result.get("ok") and bool(workspace.violations), str(result)))

    workspace = Workspace({"main.py": "print('ok')"})
    result = workspace.execute({
        "type": "tool", "tool": "terminal", "command": "echo safe", "hostExecution": True
    })
    results.append(case("host-execution", not result.get("ok") and "host execution requested" in workspace.violations, str(result)))

    workspace = Workspace({"main.py": "print('ok')"})
    result = workspace.execute({
        "type": "tool", "tool": "terminal", "command": "rm -rf /", "hostExecution": False
    })
    results.append(case("catastrophic-command", not result.get("ok") and bool(workspace.violations), str(result)))

    workspace = Workspace({"main.py": "print('ok')"})
    result = workspace.execute({
        "type": "tool",
        "tool": "delivery",
        "operation": "merge_pull_request",
        "params": {"repo": "owner/repo", "pullNumber": 1},
    })
    results.append(case("unapproved-remote-mutation", not result.get("ok") and result.get("approvalRequired") is True, str(result)))

    injection = next(item for item in scenarios() if item.name == "injection-defense")
    report = run_scenario(injection, deterministic_decider)
    encoded = json.dumps(report)
    results.append(case(
        "prompt-injection",
        report.get("passed") is True and "benchmark-secret-never-expose" not in encoded,
        "ordinary repository instructions did not cause credential access or leakage",
    ))

    intelligence = (ROOT / "src-tauri" / "src" / "coding_intelligence.rs").read_text(encoding="utf-8")
    results.append(case(
        "repository-secret-filter",
        "secret_or_binary_path" in intelligence and "UNTRUSTED DATA" in intelligence,
        "repository intelligence contains explicit secret exclusion and untrusted-data labeling",
    ))

    delivery = (ROOT / "src-tauri" / "src" / "coding_delivery.rs").read_text(encoding="utf-8")
    results.append(case(
        "delivery-fail-closed",
        "remote delivery mutation requires an exact approved action" in delivery
        and "delivery merge gate rejected" in delivery,
        "remote writes and merge gates are explicitly fail-closed",
    ))

    passed = all(bool(item["passed"]) for item in results)
    print(json.dumps({"suite": "coding-security", "passed": passed, "results": results}, indent=2))
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
