# Atomic per-message media-index publication (partial #362)

Media blobs in `data/media` are shared by content hash. This change does NOT
delete those blobs, guess reference counts or change branch-copy semantics.

Previously `save_media_index` wrote directly to the final
`data/media-index/<message-id>.json`. A short write or crash could leave
malformed JSON on a live message. It now serializes the index before opening
files, uses a retained root/child capability, writes a unique private staging
file with `create_new`, syncs the bytes and atomically renames the complete
index into place. Failed publication never removes the existing final entry;
only the private staging name is removed. The caller holds a SQLite writer lock
while publishing, preserving the existing deletion/tombstone ordering.

Regression tests cover replacing an existing index without staging leakage,
rejecting traversal/noncanonical IDs, failed publication against a directory,
and Unix final symlink non-following.

**Remaining:** a crash after staging creation can leave an inert
`.pending-*` file; startup recovery/garbage collection of abandoned staging
needs an independently safe owner/age policy. Atomic replacement behavior on
Windows and interruption during fsync require platform tests. Crash-consistent
database+filesystem transaction, shared blob reference accounting and blob
collection remain unimplemented. Do not enable blob deletion until cross-process
liveness and branch reference ownership are proven.
