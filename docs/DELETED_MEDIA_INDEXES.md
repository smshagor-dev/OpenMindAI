# Deleted multimodal message indexes (partial issue #362)

Migration 014 transactionally records deleted message IDs, including branch-truncation
and conversation cascades. Repository message/conversation deletion drains committed
tombstones after SQL success; Database::open retries them after startup recovery.
Cleanup uses a root capability retained for the connection lifetime and a held
media-index directory. Only canonical UUID.json entries are unlinked. Missing
entries are idempotent; unsafe paths and IO failures preserve retry evidence and
produce a warning. Cleanup never runs inside a caller's uncommitted transaction.
A SQLite writer transaction excludes cooperating publishers/deleters across
connections. Updated multimodal publication obtains that same writer lock, checks
message existence, and holds the lock until the index has been written.

The old SQL deletion behavior is reproduced in the fixtures: deleting the message
leaves its index on disk until the new drain runs. Nine actual production-module
Linux tests pass with real migration 001/014 SQLite tables/triggers: direct delete,
cascade, rollback, restart/recreated index, live reused ID, invalid ID, directory
collision/retry, outside-parent/final symlink, and another connection's writer lock.
Focused Clippy -D warnings, formatting, locked root metadata and diff checks pass.
A native Database/ChatRepository deletion/reopen test is added but was NOT locally
run; full app and Windows/macOS CI remain required. No desktop smoke test.

## Remaining work and retention limits

This is deliberately partial; keep #362 OPEN. Content-addressed image blobs remain
on disk, including shared images and unreferenced images. Safe reference accounting,
legacy orphan/index discovery, failed-upload blob rollback and branch-copy media
semantics still need implementation/tests. Old deleted indexes without tombstones
are not discovered here. Other delete entry points are captured by the trigger but
may await restart for cleanup if they do not invoke the repository drain methods.

Tombstones are retained to retry late publication by older processes and crashes;
they contain IDs only but grow indefinitely, and draining currently scans them all
under a writer lock. This is not a scalable final garbage collector. One unsafe
ID or IO error stops a drain; startup continues and later drains retry. A warning
is not a UI guarantee that all evidence was removed. No secure erase, backup/snapshot
scrubbing, hardware power-loss or malicious same-user filesystem relocation promise.
The root must be trusted when acquired; broader #394 containment work remains.
