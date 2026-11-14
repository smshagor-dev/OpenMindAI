# Portable output containment: paused checkpoint for #394

Implemented:

- resolve_relative rejects traversal, Windows prefix/root components and existing
  symlink/junction ancestors that canonicalize outside the selected root. Missing
  tail components are supported; dangling links fail closed. Internal links can
  still resolve. This is a compatibility/path check, not a race-safe file open.
- New text artifacts use write_new_relative: cap-std opens the parent beneath a
  root directory capability, holds that parent handle, then writes/syncs an exclusive temporary file and renames it within that
  capability after checking the final destination. A final link or existing file is not truncated. Failed writes
  clean up their newly created output through the same confined handle.
- Actual-open tests cover path replacement after successful validation and root
  pathname replacement after its capability was acquired. Relative internal links
  work; cap-std rejects absolute links even if their target is inside the root.

Additional backup checkpoint (2026-11-14):

- SQLite VACUUM now targets a private OS temporary directory. The selected root
  is opened as a capability first; backups/ is opened beneath it and held through
  exclusive pending-file creation, streaming copy, sync, rename and cleanup.
  Stable external links and parent/root pathname replacement are exercised by
  the actual writer. Failed streaming copies clean through the same held handle.
- Existing frozen-time/concurrent/collision backup cases and committed WAL data
  are covered. Ten focused Linux tests and strict focused Clippy passed locally;
  native Windows/macOS tests are pending. See CONFINED_DATABASE_BACKUPS.md.

Not complete; do not close #394:

- PDF/DOCX exports, external media renderers and other
  path-based writers still need capability-safe staging/publication. Their path
  resolution now rejects stable external links, but it is not a TOCTOU defense.
- The configured root/ancestor identity must remain trusted until its capability
  is opened. The manager does not retain a lifetime root handle yet.
- Output cleanup/publication and later path-based reads need further review.
- Filesystem and database publication are not a joint transaction (#353).
- Native Windows junction behavior remains for CI. No second-account attacker,
  real model, physical removable-drive or visual desktop test is claimed.

Tests use production modules and disposable real files. The focused helper suite
has four Linux tests; the broader source harness additionally exercises actual
PortableRootManager, artifact, model, database and network modules. Two existing
container disk-visibility tests fail on unchanged main and are excluded from the
broader filtered run. Full local Tauri compilation lacks Linux GUI dependencies.

Paused checkpoint: all implementation changes are committed to the remote branch;
no branch writer remains active. A follow-up may resume this issue after checking
for newer branch activity. Continue by migrating the remaining writers, adding
real native junction/path-replacement checks and reviewing retained root identity.
