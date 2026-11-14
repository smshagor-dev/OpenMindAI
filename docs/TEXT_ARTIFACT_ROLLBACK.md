# Text artifact save and interruption recovery (#353)

Text export validates owner/message and inserts ready metadata inside an IMMEDIATE
SQLite transaction before opening output files. It syncs an exclusive pending file,
renames through the held parent capability, then commits SQLite. Ordinary I/O/DB
errors remove only owned files; cleanup errors are surfaced. Cross-chat message IDs
and collisions are rejected. Other connections cannot see ready metadata before commit.

## Durable intent and cross-process ownership

Before output creation, a private 0600 JSON intent is written/synced as UUID.new,
renamed to UUID.json and its directory synced on Unix. It records artifact ID and
relative output path, never text content. The journal lives in generated/files/
.text-export-journal. An OS exclusive writer.lock is held from before SQLite BEGIN
through publication/commit/intent cleanup. Recovery takes that same lock then a
SQLite IMMEDIATE transaction, so it cannot clean a live writer's output.

Database::open runs recovery after migrations. An intent without a committed row
removes the pending/final output through root-confined directory capabilities. A
matching ready row preserves final bytes. Conflicting identities, malformed/oversized
intents, unsafe paths and cleanup errors preserve evidence for review and log a
warning without preventing database startup. UUID.new files can be discarded because
no output is created before intent promotion. Interrupted recovery is idempotent.
Successful exports remain successful if only post-commit intent cleanup fails;
recovery later keeps the committed file and removes the stale intent.

## Validation

Fourteen actual-module Linux tests pass, including a child process paused and killed
at intent, write, rename and SQLite commit checkpoints. Pre-commit interruptions
recover to no artifact row/output; post-commit interruption preserves exact bytes.
A second process proves the writer lock cannot be acquired while the child is alive.
Recovery is run twice to check idempotence. Other tests cover real deferred-FK commit
failure, injected partial storage-full/permission errors, unwind, ownership, collision,
parent replacement, visibility, malformed journals and conflicting registered IDs.
Focused Clippy -D warnings, app/harness formatting, locked app metadata and diff checks
pass. Native Windows/macOS, full Tauri/startup integration and physical desktop smoke
are pending CI. I/O faults are injected after real writes, not physical full disks/ACLs.

## Limits

This covers process termination with an intact selected root/journal; it is not a
hardware power-loss guarantee. Unix directory sync is attempted; Windows directory
flush, removable media/cache ordering, damaged/missing journals, externally moved
roots or directories and same-user file mutation are not fully controlled (#394).
Advisory locks require cooperating writers and filesystem lock support. Locking and
large text I/O block a SQLite writer; no new quota, timeout or cancellation is added.
Empty reservation directories may remain. Malformed/conflicting intents are kept,
not automatically quarantined; a later recovery can retry after the underlying issue
is resolved. Existing orphan files created by older versions without intents cannot
be safely attributed and are not deleted. No schema or root dependency version change.
Keep #353 open until current-head native/integration validation is satisfactory.

Paused checkpoint: code/tests published, no writer remains active after this run.
