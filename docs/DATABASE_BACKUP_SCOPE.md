# Database-only snapshots and portable workspace retention

The Maintenance screen's **Create Database Snapshot** action uses SQLite
`VACUUM INTO`. It preserves the database's committed tables (including chat
records and settings), not an archive of the OpenMindAI portable workspace.

**Not included:** uploaded images and other external media in `data/media`,
per-message indexes in `data/media-index`, `generated` outputs, local
datasets, model files, and other files external to SQLite. A restored database
may reference missing external files.

For an independent full workspace copy, close OpenMindAI first and back up
the complete selected portable storage root. Retain the directory layout.
Do not overwrite a live database. Confirm the copied data can be restored
in a disposable location before relying on it.

Backups returned by the Maintenance API expose
`scope: "databaseOnly"`; the UI marks each entry and does not turn
unknown future scope values into an implicit "full backup". Only managed
`openmindai-*.db` files appear in the database snapshot listing.

## Validation

Backend regression tests assert the scope tag in both create/list results and
exclude unrelated/unmanaged files. Node tests cover the user-visible scope
and recovery guidance and unknown-scope behavior. Full portable archive
creation/restoration, file-reference reconciliation, encryption, secure erase,
and a desktop restore smoke test are **not** implemented by this change.
