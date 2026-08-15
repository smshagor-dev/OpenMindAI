# Model Management

## The default model

OpenMindAI v1 ships against exactly one AI model:

- **Qwen3 4B**, `Q4_K_M` quantization, from
  [`Qwen/Qwen3-4B-GGUF`](https://huggingface.co/Qwen/Qwen3-4B-GGUF) on
  Hugging Face.

It's downloaded automatically during first-run setup — never bundled in the
Git repository or the installer, since it's a multi-gigabyte binary asset.
It's installed to `OPENMINDAI_ROOT\models\llm\qwen\qwen3-4b\`.

Model availability or the specific build may change over time through the
model catalog system described below — this document reflects what v1 ships
with today.

## Where models are tracked

Two systems work together, and it's worth understanding how they relate
since they're not the same thing:

1. **The model catalog** (`src-tauri/model-catalog.json`, loaded by
   `model_catalog.rs`) is a small, source-controlled, versioned list of
   recommended models — id, name, family, the Hugging Face repo to fetch
   from, quantization, hardware requirements (minimum RAM/VRAM), and
   capabilities. It's used to check hardware compatibility and report
   whether an update is available. It is **not** fetched from a remote
   server in v1 — it ships compiled into the app, matching the "no second
   model in v1" scope: a real multi-entry, remotely-updatable catalog is
   future work, not something worth half-building for a catalog that has
   exactly one entry today.
2. **The model downloader** (`model_download.rs`) does the actual
   download/verify/install work for the catalog's required entry — HTTP
   range-resumable downloads, checksum verification against Hugging Face's
   metadata, GGUF header validation, and free-space checks. It reads the
   repo and quantization to fetch from the catalog's required entry (so
   that detail lives in exactly one place), rather than hardcoding a second,
   possibly-drifting copy of "which model."
3. **The model registry** (`model_registry.rs`) is more general — it scans
   your models folder for any `.gguf` file (not just Qwen) and registers
   whatever it finds into the local database, inferring family/quantization
   from the filename when no manifest is present. This is what powers the
   Models page's list of installed models.

Because v1's catalog has exactly one entry, and that entry is the model the
app already ships against, "update available" is honestly always `false`
today — not a bug, just the correct answer until a second catalog entry
exists (see the code comment in `model_catalog.rs` if you're looking at the
source).

## Verification

Every model download is checksum-verified against Hugging Face's published
SHA256 (when available) and validated as a real GGUF file (magic bytes,
minimum plausible size) before being marked ready. An already-installed
model is re-checked (size, then checksum) the next time a download is
requested for it — if it's been corrupted or truncated since it was
installed, OpenMindAI notices and re-downloads it rather than silently
trusting a bad file forever.

## Model updates

If a model update becomes available (once the catalog has more than one
entry/version to compare against), OpenMindAI checks whether your hardware
and free disk space are compatible before recommending it, and — depending
on your Settings → Updates preferences — either notifies you or downloads
it automatically. Large model downloads are **not** enabled automatically by
default; you opt in under Settings → Updates → "Automatically download
large AI model updates."

See [docs/UPDATES.md](UPDATES.md) for the update system more broadly.
