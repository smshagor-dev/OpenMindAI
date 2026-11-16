# Immutable Hugging Face model downloads

OpenMindAI downloads multi-file model packages (checkpoint, voices and config)
from Hugging Face. The Hub's `main` branch can move between a metadata
request and the final file transfer. Requesting `/resolve/main/<file>`
after inspecting metadata can silently mix revisions, especially for small
Git-tracked config files without upstream LFS SHA-256.

## Download rule

- Resolve the primary model metadata and require its full 40-character
  hexadecimal Git commit SHA. Missing or malformed revision IDs fail
  before any model download starts.
- Download the primary checkpoint from `/{repo}/resolve/{commit}/{file}`
  instead of `/resolve/main/`.
- For config, voice and other dependencies in the **same repository**, ask
  the Hub for the metadata at the primary model's commit using
  `/api/models/{repo}/revision/{commit}?blobs=true`. Reject a response
  whose reported commit differs from the expected snapshot, then download
  the dependency using that exact commit URL.
- For dependencies from separate repositories, validate each repository's
  own metadata commit SHA and use its corresponding immutable download URL.
- Keep the existing file-size, directory-boundary, resume, SHA-256,
  package-manifest and verification-state checks intact.

The package-manifest's `sourceUrl` and primary model manifest's `repoSha`
record the resolved commits for traceability; the existing partial download
namespace already includes the full source URL, so a new revision has a
different partial-download identity.

This prevents mismatched versions caused by moving Hugging Face branches. It
does **not** make remote model bytes inherently trustworthy when the publisher
does not supply an independently trusted digest or signature. The real Kokoro
regression uses externally pinned checkpoint/voice/config hashes as a second
independent safety check.

## Verification

The Rust unit tests reject missing, shortened, nonhex or path-like
revision identifiers and validate pinned voice/config source URLs.
The GitHub `Kokoro Real Voice Inference` workflow also runs the genuine
Model Download Manager installation and non-silent WAV synthesis path, while
the ordinary CI verifies the desktop builds on Windows, Linux and macOS.

Do not merge this change unless the real Model Download Manager installation,
actual Kokoro speech, CI, Security/CodeQL and Release Readiness pass.
