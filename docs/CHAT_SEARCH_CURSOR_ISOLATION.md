# Chat search cursor isolation (#398)

The server-side paginated search already avoids loading all message bodies into
the renderer. However, a continuation cursor previously contained only
timestamp and conversation ID; reusing it after changing the search term
could silently omit otherwise matching conversations.

Continuation cursors now contain the timestamp, ID and SHA-256 of the exact
trimmed search term. The backend rejects cross-query cursors and malformed
extra fields rather than silently returning incomplete results. The search
text itself is not serialized in the cursor. Regression tests cover
same-query continuation, cross-query rejection and forged extra fields.

This is a partial correctness change, not a complete resolution of #398.
Remaining: large-history performance benchmarks, incremental query cancellation,
and optimizing short substring searches. Existing cursors are ephemeral and
will be invalidated by this change; the UI resets its cursor on each new query.
