# Writable handles for workspace replacement sync

Native Windows runs on #505 and #506 exposed seven workspace/patch transaction
Access Denied failures after the Rust Clippy prerequisite was repaired. Existing
replacement saves copied permissions to the staged file, reopened it read-only,
and called sync_all. Windows FlushFileBuffers requires GENERIC_WRITE:
https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers

Replacement now opens the staged file for read/write before copying permissions,
sets permissions through that same handle, syncs it and drops the handle before
rename. Opening before changing permissions also supports modes that remove later
write access. The original target is still moved only after this preparation
succeeds. Existing hash checks, journal/rollback/recovery and rename durability
logic are unchanged; permissions retain Rust std semantics, not a new full ACL,
owner or extended-attribute preservation guarantee.

A dedicated source-module harness runs all transaction regressions on Linux,
Windows and macOS. New coverage replaces an existing file and checks its data,
permissions and journal cleanup. A Windows-only OS-contract test demonstrates
that flushing a read-opened handle fails while a read/write handle can flush.
Local Linux: 16 production-module tests, focused Clippy -D warnings, harness/app
format and diff checks passed. Windows/macOS execution, full Tauri/workspace/
agent wrappers and physical desktop operation are pending native CI. Do not claim
the seven Windows failures have passed until actual current-head CI confirms it.

The exact BinaryProjectFileRow alias repair already published in #505/#506 is
included as a mechanical current-main Clippy prerequisite; there is no new row
mapping or dependency change. RustSec and the separate dataset-junction fixture
remain independent failures. #505/#506 must not be merged as validated while these
additional workspace failures are unresolved. A failed sync is not bypassed: the
transaction returns an error and keeps its existing rollback behavior.
