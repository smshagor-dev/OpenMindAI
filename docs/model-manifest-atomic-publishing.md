# Safer model package manifest publishing

OpenMindAI now writes `model-manifest.json` and `package-manifest.json`
to unique temporary files in their final directories, writes and syncs their
complete JSON content, and renames each file to its final name. A write or
rename failure removes that attempt's temporary file without truncating the
previous live manifest. The existing tests also verify that replacing a
previous manifest succeeds and that an obstructed destination is preserved.

The primary model manifest is published **only after** the Model Download
Manager finishes required dependency downloads and passes the cancellation
check. Failed or canceled dependency installation must not publish a newly
downloaded checkpoint as a successfully completed package.

This reduces corruption risk during portable-storage interruption and
process termination. Each manifest is updated atomically on filesystems
supporting same-directory rename; the pair of manifests is **not** one
filesystem transaction, and sudden power loss on every removable filesystem
cannot be guaranteed to preserve all data. Runtime model checks continue
to independently validate package files and their recorded SHA-256 digests.

## Acceptance

The Rust test suite runs the same helper on Windows, macOS and Linux.
The scheduled and PR-triggered Kokoro workflow uses the actual Model
Download Manager to install model, voice and config and then generates
authentic non-silent speech, checking both manifests and source hashes.
Merge only after those checks, Security, CodeQL and Release Readiness pass.
