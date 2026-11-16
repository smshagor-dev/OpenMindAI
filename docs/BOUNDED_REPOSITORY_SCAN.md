# Bounded repository-context discovery

Directory enumeration now consumes at most 512 entries before sorting each batch,
at most 8,000 entries and 128 opened directories per attached root, and at most
2,000 retained files. The existing depth limit remains eight. Failed iterator
entries count against the budget. Pending directory paths are bounded by the
entry budget. At an exact cap the scan conservatively reports incomplete coverage
without reading an additional entry to probe EOF.

Four fixed root guidance names are checked separately (at most five component
metadata probes), preserving root guidance even if the directory sample is full.
Probes reject symlink/reparse components. Enumerated links/reparse points and
source/guidance link files remain excluded. Unreadable/disappeared directory
entries or context files are skipped instead of cancelling unrelated context.
The returned root section reports partial coverage, scanned counts, maximum batch
occupancy and read-error count. Error messages/content are not injected as source.

Limits change discovery to a bounded filesystem-order sample sorted within each
batch. Large roots can omit relevant source and nested guidance; the warning says
coverage is incomplete. This is not a stable repository index or lazy retrieval.
It does not add descriptor-based race protection to source reads (the existing
metadata/open race remains separate), nor cancel a blocking filesystem syscall.

Validation commands:

```
cargo test --locked --manifest-path tests/repository-context/Cargo.toml -- --nocapture
cargo clippy --locked --manifest-path tests/repository-context/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path tests/repository-context/Cargo.toml -- --check
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

Twelve Linux tests passed locally against actual production context/error modules.
A 5,001-entry disposable fixture measured 512 consumed/buffered entries and 1,699
microseconds for the new scan in one local run. The old production collector,
with counters added only for measurement, consumed/buffered all 5,001 entries and
took 5,520 microseconds; the same cap assertion fails on it. These are illustrative
local timings, not portable latency/RSS guarantees. Deterministic iterator counts
also prove a million-entry producer is consumed only 512 times and a nearly spent
global budget consumes only its remaining three entries. Tests cover read errors,
missing source/directory, depth/directory limits, guidance precedence, bounded
source excerpts and symlink exclusion. Dedicated native CI additionally exercises
Windows junction exclusion using PowerShell New-Item rather than the existing
unrelated failing dataset mklink fixture.

No full local Tauri build is claimed because Linux GUI libraries are absent.
Native full-app and dedicated Linux/macOS/Windows CI are required before merge.
The pre-existing Rust dependency audit failure is unchanged.
