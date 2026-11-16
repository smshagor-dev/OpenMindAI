# Artifact deletion containment: partial #394

The SQLite artifact-delete function now captures a cap-std root directory when the
connection registers it, then opens each parent beneath that retained capability
and unlinks the final entry through the held parent. Canonicalize-plus-ambient-unlink
is removed. Replacing a root pathname after registration or a parent pathname after
open cannot redirect deletion. Missing files/parents remain idempotent; invalid paths,
external ancestor links, directories and I/O errors fail the SQL operation, preserving
its row. Final symlinks, including dangling ones, are unlinked themselves on Unix;
the target is never followed. The same callback serves DELETE/cascade and stale-image
cleanup. In-memory/nonportable DB connections retain their existing no-op callback.

Eight actual-module Linux tests passed, including real migration 005 trigger and
conversation cascade, path rejection/row preservation, parent/root replacement,
external links, missing paths, final/dangling links and directory errors. The old
extracted production callback fails the root-replacement regression: it follows the
new root pathname and deletes the outside sentinel. Focused Clippy -D warnings,
app/harness formatting, locked app metadata and diff checks passed. Three-OS focused
CI is added. Windows fixtures accept only exact OS rename-denial codes 5/32 and still
assert containment; if allowed, they substitute an actual junction. Windows/macOS,
full Tauri/database integration and physical desktop are pending native CI.

Do not close #394. Only cleanup retains a lifetime root capability here; other
writers/readers still require review. Root/ancestor identity before registration and
same-user relocation of the acquired directory object remain trust assumptions.
Absolute internal symlink ancestors may be rejected by cap-std. Malformed legacy
paths now fail closed. SQLite rollback cannot resurrect a file already unlinked if
another row or later database step fails; multi-file cascade/DB deletion atomicity,
trash/undo, power-loss durability and removable-drive semantics are not implemented.
A long-lived connection retains the original root identity even if its nominal path
moves. No recursive directory deletion or uploaded-media/index cleanup (#362) is added.
No dependency/version/schema or frontend changes; no live model/desktop smoke claimed.

Paused checkpoint: code/tests published; no writer active after this run. Check
newer branch activity before resumption; native current-head checks required to merge.
