# Runtime cancellation terminal-state race (#365)

Cancel and terminal completion now share a lock order (token, then status).
A late cancellation after success can no longer overwrite Completed with
Cancelled, while a cancellation observed before success blocks terminal
publication. Added two Rust regression tests for both orderings.

This is a narrow race fix; it does not make manifest writes transactional or
guarantee cleanup after interruption at every installation stage. Run cargo
fmt/test and desktop cancellation timing tests before merging.
