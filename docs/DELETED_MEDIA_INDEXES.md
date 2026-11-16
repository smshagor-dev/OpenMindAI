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

## Legacy recovery on database open

Before ordinary tombstone draining, startup scans canonical UUID.json names in
the retained media-index directory and checks each against live SQLite messages.
It queues missing IDs in durable tombstones in batches of 128, committing each
batch before any unlink. The existing drain then deletes only orphan indexes;
live/reused message IDs and shared image blobs remain untouched. Unknown file
names are ignored rather than treated as filesystem deletion authority. Startup
errors log a warning and can be retried on a subsequent open. Enumeration can
take time for large legacy directories; SQLite writer locks are held only for
each bounded liveness/queue batch, not the entire directory scan.

Five focused new regressions cover legacy orphan recovery and shared preservation,
rollback, multi-batch recovery/idempotence, cross-connection writer exclusion and
Unix symlink safety. These were added but not locally compiled or run; three-OS
CI is required.

## Remaining work and retention limits

This is deliberately partial; keep #362 OPEN. Content-addressed image blobs remain
on disk, including shared images and unreferenced images. Safe reference accounting,
unreferenced blob discovery, failed-upload blob rollback and branch-copy media
semantics still need implementation/tests. Pre-migration deleted indexes are now
queued at startup when enumeration succeeds. Other delete entry points are
captured by the trigger but may await restart when they do not call repository
drain methods.

Tombstones are retained to retry late publication by older processes and crashes;
they contain IDs only but grow indefinitely, and draining currently scans them all
under a writer lock. This is not a scalable final garbage collector. One unsafe
ID or IO error stops a drain; startup continues and later drains retry. A warning
is not a UI guarantee that all evidence was removed. No secure erase, backup/snapshot
scrubbing, hardware power-loss or malicious same-user filesystem relocation promise.
The root must be trusted when acquired; broader #394 containment work remains.
