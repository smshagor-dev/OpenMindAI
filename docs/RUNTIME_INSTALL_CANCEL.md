# Runtime installer cancellation (partial #365)

Runtime installation cancellation is now observed during GitHub release lookup,
archive ZIP/tar.gz source reads, staging/promotion and before manifest commit.
Release lookup uses a cancellation-select path, rather than waiting for the
network call to return. Archive extraction runs with `spawn_blocking`, so the
Tauri async executor can accept Cancel while the worker is decompressing.

ZIP and tar.gz readers check the token on every compressed source read; for
ZIP they also check seek. A canceled staging operation is cleaned up and cannot
replace a working runtime. Cancelling before extraction never starts it.
The installer reports Cancelled as soon as the command is received and avoids
publishing a manifest when cancellation is observed before commit.

Four focused regression tests cover pending lookup cancellation, reader
read/seek rejection, preserving an installed runtime when canceled before
extraction, and rejecting canceled ZIP/tar operations. Existing installation
tests receive the new token argument.

**Remaining limitations:** a cancellation arriving in the very small interval
between the last pre-commit check and filesystem manifest publication can
still race with completion. SHA-256 verification remains synchronous and does
not yet check the token during hashing. There is no promise of immediate
cancellation during one blocking filesystem operation, atomic rollback after
promotion, or hardware power-loss safety. Real slow-network/extraction desktop
timing tests and full cross-platform CI remain required.
