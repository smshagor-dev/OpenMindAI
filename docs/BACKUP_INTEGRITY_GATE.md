# Backup integrity gate (partial #381)

The database-only backup path now runs SQLite `PRAGMA quick_check` against
its private, read-only staged snapshot **before** publishing it under
`backups/`. A corrupt or non-SQLite staging file is rejected; no catalogued
backup is created. This complements the existing capability-confined atomic
publication, WAL-safe VACUUM INTO and explicit database-only UI disclosure.

A Rust regression test rejects invalid bytes and accepts a genuine generated
snapshot. It has not been executed in this run.

This does **not** add full workspace backup/restore: generated documents,
image/video/audio media, indexes and datasets remain outside the SQLite
snapshot. #381 must stay open. Full media-inclusive backup, restore validation
and interrupted-operation testing are separate follow-up work.
