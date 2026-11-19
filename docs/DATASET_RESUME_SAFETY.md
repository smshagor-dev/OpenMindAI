# Dataset download resume safety (partial #326)

The dataset wrapper previously deleted `*.openmindai-part` before calling the
shared network resume client, so a valid interrupted download always started
over. This patch retains the partial and sidecar; `net::download_resumable`
already binds sidecar metadata to the URL hash, validator and expected length,
validates Content-Range/If-Range and refuses unsafe append on mismatch.
Existing downstream SHA-256 verification still happens before installation
when an upstream checksum is available.

A new target guard rejects symlink/nonregular part files and metadata sidecars
before the HTTP operation, preserving the portable-root trust boundary.
Tests cover partial-byte retention, symlink rejection and a local HTTP 206
fixture with If-Range/Range, verified final SHA and no retained partial.

**Remaining:** proper dataset Pause/Cancel UI and active cancellation token,
a full interrupted network stream restart test (as opposed to preconstructed
valid metadata), atomic sidecar persistence and cross-process TOCTOU hardening.
This does not claim full #326 closure. Do not merge without Rust/toolchain
and cross-platform validation.
