# OpenAgent Engineering Plan

OpenAgent is OpenMindAI's local-first coding agent. It is designed to inspect real projects, make bounded changes, run permitted development tools, recover from failures, validate its work, and report evidence instead of claiming success without a passing result.

## Model baseline

The preferred model is NVIDIA Nemotron 3.5 Lightning 30B-A3B through the verified GGUF repository `ggml-org/NVIDIA-Nemotron-3.5-Lightning-30B-A3B-GGUF`. OpenAgent chooses the installed Lightning package first, then compatible Nemotron Nano packages, and finally the existing general reasoning model so lower-memory machines remain usable. The actual selected model ID is sent to the local OpenAI-compatible llama.cpp endpoint; it is never replaced by a hardcoded Qwen identifier.

## Execution architecture

OpenAgent uses a bounded inspect, act, observe, validate loop. Each model turn returns one structured JSON action. Host code validates the action before execution, records a bounded tool result, refreshes workspace context after mutations, prevents identical-action loops, stops after repeated failures, supports cancellation, and enforces a maximum step count.

The initial tool set covers directory listing, bounded file reads, text search, full-file writes, exact text replacement, directory creation, path moves, deletion, Git status, Git diff, and terminal execution. File tools are contained inside explicitly attached project roots. Absolute paths and terminal execution remain behind the separate Full PC + Terminal grant. Project files and command output are treated as untrusted data rather than agent instructions.

## Delivery stages

### Stage 1: Nemotron-powered foundation

- Brand the project coding workflow as OpenAgent.
- Prefer Nemotron 3.5 Lightning when its verified local package is installed.
- Send the selected model ID to llama.cpp.
- Preserve compatible fallbacks for machines that cannot load the 30B package.
- Keep workspace containment, cancellation, failure budgets, and validation gates.
- Add routing and regression tests.

### Stage 2: Durable run state

- Add persistent run, step, tool-call, checkpoint, and validation records.
- Resume interrupted runs without replaying completed mutations.
- Record changed files and reversible checkpoints before destructive edits.
- Expose a run timeline, token/runtime metrics, and clear failure reasons in the UI.

### Stage 3: Strong process sandbox

- Add a platform adapter for Windows Sandbox/Job Objects, macOS sandbox profiles, and Linux bubblewrap or containers.
- Default terminal execution to a disposable workspace copy with explicit mount and network policies.
- Apply CPU, memory, process, output, and wall-clock limits.
- Export reviewed patches back to the attached project instead of granting broad host access.

### Stage 4: Codex-grade editing and context

- Add patch-based edits, symbol-aware search, repository instruction discovery, ignore rules, and binary/generated-file protection.
- Build adaptive context packs from repository maps, relevant files, Git state, diagnostics, and recent tool evidence.
- Add language-aware validation profiles for Rust, TypeScript, Python, Go, PHP, Java, .NET, and mobile projects.

### Stage 5: Git delivery workflow

- Add explicit branch policy, commit composition, remote status checks, PR creation, CI monitoring, and merge controls.
- Never include unrelated user changes in an agent commit.
- Require passing local validation and repository checks before automatic merge.
- Preserve a complete audit trail for every remote mutation.

### Stage 6: Production qualification

- Run adversarial prompt-injection, path escape, symlink, archive, command injection, timeout, cancellation, and resource exhaustion tests.
- Run real-model tool-use evaluations with Nemotron 3.5 Lightning and every supported fallback.
- Validate packaged Windows, macOS, and Linux builds on clean machines.
- Publish operator documentation, privacy behavior, limitations, and recovery procedures.

## Current implementation status

Stage 1 is complete. The bounded agent loop is connected to installed Nemotron 3.5 Lightning packages, uses the selected model ID, retains compatible lower-memory fallbacks, and runs inside the attached-root file boundary.

Stage 2 is in progress. OpenAgent now persists run state, ordered tool steps, bounded results, validation state, and before/after workspace checkpoint manifests. Startup recovery marks abandoned runs as interrupted, and typed desktop APIs expose run history and step details. Safe mutation replay/resume, content-addressed reversible file snapshots, token/runtime metrics, and the visible timeline UI are still pending and must not be represented as complete. The OS-level process sandbox remains Stage 3.
