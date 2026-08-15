# Maintenance Center

Settings → Maintenance gives you visibility into, and simple recovery tools
for, your OpenMindAI installation — without needing to know anything about
llama.cpp, GGUF, or SQLite.

## System Health

A quick status line for storage writability, whether the AI runtime is
installed, whether the AI model is verified, and available disk space.

## Diagnostics

"Run Diagnostics" performs a read-only health sweep:

- Storage root is writable.
- Database integrity (`PRAGMA integrity_check`).
- Database schema version matches what this app version expects.
- An AI runtime is installed and selected for your hardware.
- An AI model is installed and ready.
- Free disk space (warns under ~2 GB).

This has no side effects — it only reports. "Repair" (below) is a separate,
explicit action.

## Repair

"Repair OpenMindAI" re-runs the same setup steps that already proved
themselves during first-run setup, only for whatever's actually missing:

- Re-creates any missing required folders.
- Re-installs the AI engine, if none is currently installed and validated.
- Re-downloads the AI model, if none is currently installed and verified.

**It never touches your conversations, database, or settings.** If
diagnostics show a healthy install, repair reports "already installed" for
each check and changes nothing.

## Backups

"Create Backup Now" snapshots your SQLite database (using SQLite's
`VACUUM INTO`, which is safe to run against a live database in WAL mode)
into `OPENMINDAI_ROOT\backups\`, timestamped. "Open Backups Folder" opens
that folder directly. Backups accumulate; OpenMindAI doesn't currently
delete old ones automatically, so periodically clean up the folder yourself
if you're creating backups often.

## Logs

"View Recent Activity" shows the tail of OpenMindAI's own structured log
(startup, setup progress, runtime/model install events, repair/backup
actions, update checks) right in the app — a quick look without leaving
OpenMindAI. "Open Logs Folder" opens the full log history in
`OPENMINDAI_ROOT\logs\`, including this rotated structured log and the raw
output of the local AI server process.

**Your conversation content is never written to these logs.** Chat messages
only ever flow into the local SQLite database, never into a log file — this
is a deliberate invariant, not an oversight.
