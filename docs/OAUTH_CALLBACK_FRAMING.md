# Desktop OAuth callback framing

Google Workspace and Microsoft/Slack/Notion/Dropbox OAuth now call one loopback
callback reader. It reads a complete HTTP request head (at most 16 KiB), requires
GET with the configured path and exact state, rejects duplicate/empty sensitive
query parameters, and keeps listening after irrelevant/invalid requests. Only a
state-bound provider error ends authorization as a failure.

Up to eight requests are read concurrently, so a stalled connection does not
block a legitimate callback. Each request head has five seconds, browser replies
have 500 ms, and at most 128 connections are accepted. The existing five-minute
overall authorization deadline remains. Dropping/cancelling the receiver aborts
pending socket tasks. Browser reply failure does not discard an authenticated
code. Responses contain no code/state and disable caching.

Google retains its existing ephemeral IPv4 loopback redirect. Other connector
redirects retain localhost/127.0.0.1 and additionally support an explicitly
configured [::1] loopback URI. localhost continues binding IPv4; dual-stack DNS
fallback and provider redirect registration are not changed.

Validation:

```
cargo test --locked --manifest-path tests/oauth-callback/Cargo.toml
cargo clippy --locked --manifest-path tests/oauth-callback/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path tests/oauth-callback/Cargo.toml -- --check
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

Eight tests import the actual production module and use real IPv4/IPv6 localhost
sockets. They cover split request lines/headers, unrelated/wrong-state callbacks,
stalled competing sockets, header/attempt bounds, EOF, provider denial, request
and overall deadlines, and cancellation cleanup. The same competing/fragmented
fixture fails against the previous production callback function extracted from
main (connection reset), and passes against the new shared receiver. An additional
full-app test verifies configured IPv6 listener binding and non-loopback rejection.
That wrapper test is pending native CI, not claimed as locally executed.

No real provider login/browser interaction or OS-level app cancellation UI was
performed locally. Full local Tauri compilation lacks GUI libraries; dedicated
Linux/macOS/Windows jobs and full native CI are the integration gates. The Windows
dataset junction fixture and Rust dependency audit failures are separate baseline
issues, not evidence that these changes passed all repository checks.
