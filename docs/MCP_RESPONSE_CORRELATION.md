# MCP response correlation

The desktop connector generates one UUID request ID and passes that same ID to
its JSON/SSE response reader. JSON responses must match the ID and contain a
JSON-RPC 2.0 result or error. SSE parsing handles LF, CRLF and CR line endings,
comments, optional initial UTF-8 BOM, data fields joined by LF, and arbitrary
network/UTF-8 chunk boundaries. Other IDs and notifications do not become results.
A matching complete event returns immediately without waiting for stream closure.

Both transports retain the 2 MiB total response budget. The existing request
30-second timeout is retained, with an additional explicit 30-second response
reader deadline. Invalid UTF-8/JSON, matching protocol errors, truncated streams,
HTTP/read failures, missing IDs and exhausted budgets fail explicitly. This does
not implement MCP initialization, session IDs, SSE reconnection/Last-Event-ID,
JSON-RPC batches or unsolicited server-initiated request handling.

Validation commands:

```
cargo test --locked --manifest-path tests/mcp-response/Cargo.toml
cargo clippy --locked --manifest-path tests/mcp-response/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path tests/mcp-response/Cargo.toml -- --check
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

The harness imports the actual production module. Nine tests include HTTP fixture
servers and byte-boundary parser regressions. The old last-data-line algorithm
selects a trailing notification and fails on multiline data in the reproduced
fixtures. Dedicated jobs run the same tests on Linux, Windows and macOS. Live
third-party MCP provider interoperability has not been tested locally. Full local
Tauri compilation requires GUI libraries unavailable in this environment; native
CI remains the integration gate. Existing Windows dataset-junction fixture and
Rust dependency audit failures are separate baseline issues.
