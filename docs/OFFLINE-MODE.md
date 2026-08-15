# Offline Mode

OpenMindAI is local-first: the AI model runs on your computer, and your
conversations are stored in a local SQLite database under your chosen
`OPENMINDAI_ROOT`. This page explains exactly what needs internet access
and what doesn't.

## Requires internet

- **First-run setup** — downloading the AI engine (llama.cpp) build for your
  hardware and the AI model (Qwen3 4B) from their official sources.
- **Application update checks** — background and manual ("Check for Updates
  Now" in Settings → Updates).
- **Model update checks** — background and manual ("Check for Model
  Updates"), and downloading a new/updated model if one becomes available.
- **GitHub account features** (if you connect one under Settings) and any
  other explicitly online integration you opt into.

## Does not require internet

- Chatting with the AI model, including reasoning/coding responses and
  streaming output.
- Conversation history — reading, searching, renaming, pinning, deleting.
- Stopping/cancelling an in-progress response.
- Local settings and your user profile.
- GPU acceleration and CPU fallback.
- The Maintenance Center's diagnostics, repair, and backup features (they
  only touch local files and your local database).

## What happens if you're offline

- **Update checks fail silently.** A failed or offline update check never
  blocks app startup, never shows an error dialog, and never delays opening
  a conversation. It just quietly doesn't find anything, and tries again
  next time. This is intentional — see `App.tsx`'s update-check effects if
  you're curious about the implementation.
- **Chat still works.** Once the AI engine and model are installed, they run
  as a local process on your machine (`llama-server`, bound to
  `127.0.0.1` only — never exposed to your network). No network round-trip
  is needed to generate a response.
- **First-run setup cannot complete offline.** The AI engine and model have
  to be downloaded once. If your connection drops mid-download, OpenMindAI
  resumes from where it left off (`.part` files + HTTP range requests)
  rather than starting over.

## Privacy

Your conversations, local database, installed models, and generated files
all live under your chosen `OPENMINDAI_ROOT` — see
[docs/PORTABLE-STORAGE.md](PORTABLE-STORAGE.md). OpenMindAI does not require
an OpenAI, Anthropic, or Google API key, a cloud account, or a subscription
for local chat. There is no telemetry — no analytics or usage data is sent
anywhere (checked directly against the code, not just claimed: there is no
analytics SDK dependency anywhere in this repository).
