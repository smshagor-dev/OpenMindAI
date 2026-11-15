# Coding agent HTTP gateway: completion response contract

Partial fix for #379. The localhost gateway at POST /v1/chat/completions
currently forwards a non-streaming request and returns exactly one JSON result.
It does not support Server-Sent Events (SSE) yet.

- Omitted stream or stream: false: existing buffered JSON completion.
- stream: true: HTTP 501 Not Implemented with a JSON error explaining that
  streaming is unavailable and that callers may retry with stream: false.
- Non-boolean stream values: HTTP 400 Bad Request with a JSON error.

The validation runs before runtime acquisition or cloud/local completion,
so a rejected SSE request cannot accidentally incur inference work. This
does not implement streaming, usage chunks, disconnect semantics for SSE, or
streaming extension compatibility. Do not advertise SSE support until
chunk framing and cancellation are end-to-end tested.

The new focused Rust regression checks the request contract and status
reason phrase. It has not been executed locally; GitHub native/OS CI and
real authenticated TCP client tests are still required.
