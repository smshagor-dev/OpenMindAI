# Model Download Manager request coordination

The desktop application exposes a shared Model Download Manager to multiple
views. Previously a second install request could replace the first request's
status and cancellation token while the first download was still writing to
its package directory, causing confusing progress, cancellation and
manifest state.

The manager now uses one shared, non-blocking install permit across its
clones. A second install request receives a clear **already in progress**
error without modifying the active request's status or cancel token.
The active request holds the permit while resolving remote metadata,
downloading the primary checkpoint and its dependencies, publishing
manifests, and recording final status. On any normal return, the permit
is released for subsequent requests.

A pause or cancel request when no download token is active now returns
the current status without rewriting it to `Cancelled` or
`PausedInterrupted`. This avoids falsely changing a successful install
after completion.

## Scope and validation

The Rust regression tests simulate two views sharing one manager instance,
verify that the second request cannot displace the first, and verify that
idle pause/cancel operations preserve both queued and completed states
without accessing the network.

This guards concurrent calls to the **same manager and its clones**. It
is not a cross-process file lock: two separate OpenMindAI processes
installing to the same portable root remain a separate concurrency problem.
The existing Model Download Manager real Kokoro install-and-speak workflow,
three desktop voice synthesis jobs, and Windows/macOS/Linux CI remain the
merge gate.
