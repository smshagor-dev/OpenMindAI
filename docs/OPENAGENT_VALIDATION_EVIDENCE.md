# OpenAgent validation invocation evidence

A terminal result can satisfy the post-edit validation gate only when the host
executor recognizes one direct validation invocation, observes exit code zero,
and does not time out. The controller consumes that host-created receipt; it no
longer searches the model's command text for a test-command substring.

Recognition tokenizes a deliberately restricted unquoted ASCII argv syntax.
Known executables/subcommands include Cargo checks/tests/builds, npm validation
scripts, Python's pytest module, Go tests/vet and the other explicit runner
patterns in validation_command.rs. Shell comments, expansion, pipelines,
redirection, command chains, executable-path lookalikes and generic wrappers are
ineligible. Common help/version/collection-only, dry-run, skip-test, exclusion and
mutation flags are also rejected. Unsupported commands may still execute under
the existing approval policy but cannot clear validation-required state.

Durable steps retain the validated command in the existing validation_command
field. Host results retain the bounded transcript with invocation eligibility,
backend, isolation/network mode, cwd, exit code, timeout and stdout/stderr. Failed
and timed-out executions follow the existing failed-step path; echo commands do
not receive a validation receipt even when their output contains a runner name.
No schema or dependency changes are needed.

This is execution evidence for an eligible invocation, not forensic attestation
of arbitrary project scripts or a count of tests actually run. PATH resolution,
project-owned npm/composer scripts, local tool configuration, aliases inside the
underlying execution environment and tool-specific semantics remain trusted.
A recognized runner can legitimately find zero tests. Bounded output can omit
later details; no automatic assertion that every project test ran is added.
Quoted arguments, absolute executable paths, gradlew, npx, shell wrappers and
unlisted validation scripts are conservatively excluded from gate credit.

Six production-helper regressions cover recognized argv, echo/printf/comments,
wrappers/lookalikes, shell syntax, help/collection/skip modes and host exit/timeout
status. One test runs an actual harmless host echo process; replacing recognition
with main's old substring function makes it fail. These tests do not execute real
project test suites or the complete Tauri agent loop. Full local Tauri compilation
requires missing Linux GUI libraries; native CI must compile the integration.
