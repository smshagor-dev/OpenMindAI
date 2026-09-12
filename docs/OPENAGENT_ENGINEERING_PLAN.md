# OpenAgent Engineering Plan

OpenAgent is OpenMindAI's local-first coding agent. It is designed to inspect real projects, make bounded changes, run permitted development tools, recover from failures, validate its work, and report evidence instead of claiming success without a passing result.

## Model baseline

The preferred agent model is NVIDIA Nemotron 3.5 Lightning 30B-A3B through the verified GGUF package configured in the OpenMindAI model catalog. OpenAgent prefers an installed compatible Nemotron package and retains lower-memory fallbacks. The selected model ID is passed to the local OpenAI-compatible llama.cpp endpoint rather than being replaced by a hardcoded model identifier.

## Execution architecture

OpenAgent uses a bounded inspect, plan, act, observe, validate, review, and delivery loop. Model output is parsed as structured actions. Host code owns policy, workspace containment, approvals, durable state, checkpointing, validation evidence, sandbox selection, delivery gates, and failure budgets.

Repository content, command output, worker analysis, and connected-service responses are treated as untrusted evidence. Repository guidance is scoped and bounded and never overrides user intent, secret handling, sandbox restrictions, approval policy, or other host safety controls.

## Production capabilities

1. **Strong isolated execution.** Linux uses bubblewrap when available, macOS uses the system sandbox profile backend when available, and Windows uses the disposable Windows Sandbox microVM. Isolated execution clears inherited host environment data, constrains writable workspace access, disables networking, applies process/output/time/resource guards, and does not silently fall back to host execution.
2. **Patch-based atomic editing.** Coordinated multi-file patch transactions are preflighted, journaled, stale-file protected, and rolled back on failure. Single-file workspace writes use the same transaction boundary where applicable.
3. **Symbol-aware navigation.** LSP-backed symbols, definitions, references, hover, diagnostics, and call hierarchy are available when a trusted language server can run. Navigation falls back through Tree-sitter AST analysis before bounded lexical fallback for supported operations.
4. **Repository instruction discovery.** Root guidance and scoped nested `AGENTS.md` files are discovered with precedence rules, size/file-count limits, secret filtering, symlink avoidance, and explicit untrusted-data boundaries.
5. **Automatic context selection and compression.** Goal terms, repository evidence, recent execution results, instructions, conversation context, and workspace state are ranked and compressed against the active model context budget while preserving recent failure evidence and host delivery policy.
6. **Interrupted-run resume and recovery.** Runs, ordered steps, plans, validation state, approvals, checkpoints, restore events, parent/child continuation links, and recovery evidence are durable. Resume seeds explicitly prevent replaying successful mutations. Checkpoint restore verifies containment, symlinks, digests, after-state conflicts, and rollback behavior before changing files.
7. **Git branch, commit, PR, CI, and merge automation.** Delivery tools cover branch creation, bounded multi-file commits, PR create/update/read, workflow/check/job/log inspection, bounded reruns, and merge. Merge is fail-closed: it requires exact approval, successful local validation evidence, observed acceptable repository check conclusions, an open/unmerged PR, the full observed 40-character PR head SHA, and the same SHA as a server-side GitHub merge precondition.
8. **Approval and risk policy engine.** Read-only work remains low-friction; workspace writes, destructive actions, host execution, compound shell syntax, and remote mutations are classified separately. Exact approvals are action-hashed, persisted, single-use, auditable, and inherited only through explicit run continuation lineage. Shell chaining, pipes, redirects, substitutions, and multiline commands cannot inherit a trusted read-only prefix.
9. **Parallel sub-agents.** Two to four bounded read-only workers can analyze implementation, validation/security, architecture, and delivery evidence concurrently. Workers cannot mutate files, call tools, run commands, access credentials, or change Git state; the parent remains the sole mutator.
10. **Token, cost, and runtime metrics.** Durable metrics account for prompt/completion tokens, model time, tool time, worker calls, local cost estimates, hardware metadata, and budget stop reasons.
11. **Visible timeline and recovery data.** Plans, events, approvals, tool steps, checkpoints, metrics, restore outcomes, and continuation links are persisted through typed desktop APIs for the coding-run UI and recovery workflow.
12. **Nemotron end-to-end qualification.** The repository includes deterministic workspace evaluation plus a live Nemotron qualification harness and workflow for a local OpenAI-compatible endpoint. The model is never granted benchmark host authority during qualification.
13. **Adversarial security testing.** The coding security suite covers path traversal, credential-like file access, host execution, catastrophic commands, prompt injection, exact approval boundaries, compound-shell bypass attempts, stale PR-head merge protection, server-side merge SHA enforcement, and native-worker Vulkan build-contract assertions.

## Native Vulkan runtime packaging

The Windows Vulkan package workflow builds the pinned shared llama.cpp runtime, stages ABI-locked runtime files, runs a CXX initialization probe, builds the persistent Rust native worker against the same dynamic runtime contract, and validates the packaged runtime on a fresh runner. The persistent worker build uses explicit import-library search paths, restores its build working directory, shares the strict ABI/portable build environment, writes to a deterministic Cargo target directory, and verifies the expected worker executable before artifact staging.

## Remaining verification before merge

The code-side pending hardening work is implemented on the feature branch, but this document does not claim that the branch has passed the user's local validation or a fresh post-change CI run. Before merge, run formatting, Rust checks/tests/Clippy, frontend lint/build, deterministic coding evaluation, the adversarial coding security suite, the native worker Windows build against the pinned dynamic runtime, and the Native Vulkan packaging workflow. Any failing check must be diagnosed and fixed without weakening the corresponding guard.
