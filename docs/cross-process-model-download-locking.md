# Cross-process model installation coordination

OpenMindAI can use the same portable root from two independent desktop
processes. The in-memory one-install permit shared by the views in a single
process cannot prevent a second process from writing the same model,
dependency or manifest paths.

Each Model Download Manager now takes a **nonblocking exclusive OS file
lock** on `<portable-root>/.openmindai-model-install.lock` before changing
download status, cancellation state, partially downloaded data or installed
model manifests. The lock is held by the file handle until the full install
returns. A second process trying to install to the same root gets a clear
message rather than overwriting files or silently stealing progress.
Independent roots can still install separately.

This uses the Rust standard library `File::try_lock` API (stable since
Rust 1.89) and therefore does not introduce another package dependency.
Windows uses `LockFileEx`; Unix uses operating-system file locks. Crashes
or other premature process exits automatically release the handle's lock,
so no PID-based stale-lock deletion is needed.

**Do not delete or replace the lock file.** Its persistent path and inode
are part of the coordination protocol. The code rejects an existing
non-regular lock destination or symlink, checks root containment, and
fails closed when a filesystem does not support an OS lock. A lock file
left after a crash is normal and can be reused.

## Coverage and limitations

Rust regression tests verify that two separately instantiated managers
pointing at the same root cannot both hold the lock, that a different root
remains independent, that a released lock is reusable, and that a refused
install does not overwrite an idle manager's status. An invalid lock path
must be rejected without modifying it.

This is an advisory protocol. Any unrelated writer that ignores this file
lock can still modify assets, and a network/removable filesystem may have
OS-specific locking behavior. All cooperating OpenMindAI model installs
must use the same protocol and must keep the lock file persistent.
This also does not claim power-loss transactional updates across both
model manifests. Checkpoint, voice and config hash validation remains
independent of the lock.

The real Model Download Manager install + speech, Windows/macOS/Linux
Rust tests, Security and Release Readiness are required before merge.
