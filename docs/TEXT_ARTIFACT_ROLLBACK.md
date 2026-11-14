# Text artifact rollback: partial #353

The text-export command now validates the owning conversation/message and inserts
its metadata inside an IMMEDIATE SQLite transaction before opening output files.
A message from another conversation is rejected. The ready row is invisible to
other connections until commit. Bytes are written/synced to an exclusive pending
file, then renamed through a retained parent directory capability. On ordinary
write, sync, rename or SQLite commit errors, rollback removes only the pending or
final file owned by this operation through that same capability. Cleanup failure
is returned with the original error instead of silently declaring rollback complete.
Unwinding also attempts cleanup. Successful and empty exports retain exact bytes.

Ten focused production-module Linux tests pass: missing owner/message, cross-chat
message, rejected INSERT, a real deferred-FK COMMIT failure after publication,
injected partial storage-full and permission errors, unwind, collision preservation,
unowned pending entry preservation, parent replacement and pre-commit row visibility.
The fault writer writes actual bytes before returning the injected I/O error;
physical disk-full and ACL changes were not exercised. A disposable SQL/file fixture
reproduces the old file-first order: failed FK INSERT leaves one orphan file.
Focused Clippy -D warnings, app/harness rustfmt, locked app metadata and diff checks
pass. Dedicated three-OS harness CI is added. Native Windows/macOS and full Tauri
command integration are pending CI; no desktop smoke was run.

## Still incomplete: do not close #353

SQLite and the filesystem do not share a commit primitive. SIGKILL, abort or power
loss between pending-file creation/rename and database commit can still leave an
orphan or pending file. This change has no durable intent journal or startup recovery;
catch_unwind is not an abrupt-process test. A follow-up must journal before creating
files, recover under a cross-process ownership policy and test child-process kills
at each boundary. Empty UUID reservation directories can remain on failed attempts.
SQLite/OS errors that also prevent cleanup are surfaced, not guaranteed recoverable.

Root ancestry is trusted until capability acquisition; same-user final check/rename
races and relocated directory objects have the same limitations as other portable
writers (#394). Directory sync is attempted on Unix; no Windows power-loss durability
guarantee. IMMEDIATE holds the SQLite writer lock during text I/O; large text exports
can delay concurrent writes. No content quota was introduced. No database migration,
new dependency version or frontend change is included.

Paused checkpoint: implementation and tests are committed/published; no writer
remains active after this run. Check newer branch activity before resuming.
