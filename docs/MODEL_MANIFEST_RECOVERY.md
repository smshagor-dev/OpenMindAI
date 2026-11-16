# Model manifest recovery

Main already published model/package manifests through synced temporary files.
The remaining failure was recovery: missing, damaged or interrupted metadata
could still block a complete local checkpoint. Both manifest writers now first
publish a small `.json.recovery` record, then the live JSON. Each publication
uses an exclusive UUID temporary file, write-all, file sync and same-directory
rename. Unix also syncs the parent directory. Failed writes remove their own
temporary file and leave an existing complete live record intact.

Media preflight and dependency validation can restore missing/invalid metadata
from that record. A different recovery record may be the newer completed stage
of an interrupted publication; it is considered before retaining older live
metadata. Recovery checks catalog identity, filenames, containment, sizes and
fresh SHA-256 digests of installed assets before promoting the record. Primary
recovery also verifies it describes the selected model file. A bad candidate
cannot replace a usable live manifest. Neither record is used to assert remote
authenticity that was absent in the original installation; Unverified remains
Unverified when no upstream digest was available.

Normal matching live/recovery records use existing quick metadata checks.
Full file hashing occurs during recovery or explicit dependency validation;
this is not an every-refresh hash of multi-gigabyte files. Recovery is synchronous
and can take time on large checkpoints. Issue #413's general distinction between
quick health checks and full integrity verification remains separate.

The records are capped at 64 KiB, read with byte limits, and reject final-component
symlinks/special files. Broader race-resistant portable-root path opening is
tracked in #394. This change requires trusted ancestor directories and local
filesystem rename semantics; it does not claim protection from same-user path
replacement or physical power-loss testing. Windows flushes files and renames
records; no directory-flush guarantee is claimed there.

Existing installations without a recovery record remain readable. If both
records are missing/invalid, recovery fails closed rather than inventing digest
or repository metadata. Use the existing model install/repair flow: it resolves
upstream metadata and reuses complete matching local files after verification,
without requiring a checkpoint re-download. Offline reconstruction is limited
to installations with enough surviving recovery metadata. Registry discovery
alone does not initiate primary recovery; media preflight does.

Regression tests exercise both manifest names, partial writes with injected
StorageFull errors in either publication stage, interrupted publication, missing
and truncated live records, rejected malformed/oversized writes, primary and
dependency same-size corruption, and the real media-preflight recovery path.
Fixtures contain small synthetic files, not usable model checkpoints. Tests do
not exhaust the host disk or call a real model/provider.
