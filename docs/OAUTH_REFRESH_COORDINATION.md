# OAuth refresh coordination

Each credential store has one process-wide async refresh gate: Microsoft, Slack,
Notion, Dropbox, Google Workspace and legacy Connected Google. The two Google
stores deliberately have separate gates because their credentials are distinct.
The gate covers the expiry/credential re-read, refresh request and persistence.
Overlapping waiters receive the completed attempt's token or failure. A later
request re-reads persisted state, and can retry after a failed refresh.

A waiter has a 95-second acquisition deadline. Provider refresh HTTP calls have
30-second request timeouts. Cancelling a waiter removes it from the queue;
cancelling the owner releases its guard so subsequent calls can proceed. This
is process-local coordination, not a cross-process or account-switch lock.
Connect/disconnect and configuration changes are not serialized by these gates.
Cancellation cannot undo a refresh already committed by a provider: if its
response is lost after rotation, signing in again can still be necessary.

Google Workspace persists its token bundle as one existing secret-store value.
The other connectors retain separate secret and expiry records. A returned,
nonempty replacement refresh token is saved before access/expiry metadata, so a
later metadata write failure does not discard that replacement. These separate
writes are not an atomic transaction with the OS keyring or remote provider.
No credential is logged or placed in a diagnostic.

Regression tests in `src-tauri/src/oauth_refresh.rs` exercise the production gate:

- Two expired batches of 24 simultaneous callers against a local rotating-token
  HTTP server produce exactly one refresh per batch and reuse the saved rotation.
  A disposable mutation removing the gate fails with `refresh token reused`.
- Overlapping failure is shared; a later request can retry.
- Cancelled ownership releases the gate; independent stores continue.
- Waiting is bounded, verified with a paused Tokio clock.
- The two Google credential stores do not share in-flight results.

The HTTP fixture uses synthetic credentials and an in-memory persistence adapter.
It does not call real providers or exercise platform keyring failure modes. Native
CI compiles the real connector integrations and runs these regressions. Local
focused tests import the real coordinator and AppError modules; full local Tauri
compilation requires Linux GUI libraries unavailable in the current environment.
