# Architecture

OpenMindAI is organized around one invariant: application-owned data is resolved beneath OpenMindAI Root.

The Rust backend owns path resolution, database access, hardware inspection, model discovery, and future runtime process management. The React frontend calls typed Tauri commands and does not construct storage paths.

Current services:

- `PortableRootManager`: resolves root, creates directories, validates writability, rejects traversal.
- `Database`: opens SQLite, enables WAL and foreign keys, runs migrations, creates the local profile.
- `ChatRepository`: persists conversations and messages, including streaming/interrupted assistant messages.
- `HardwareProfiler`: reports real CPU/RAM values and conservative backend availability.
- `ModelRegistry`: scans the project-root `models` directory for `.gguf` files and registers them in SQLite.
- `LlamaRuntimeManager`: establishes the process boundary for llama.cpp without faking inference.
- Realtime UI: the frontend can optionally connect to a user-configured local Socket.IO endpoint for typing/search/research activity events. It is disabled by default and does not expose the app over the network.

Phase 1 focuses on portable storage, persistence, and the desktop shell. Phase 2 extends this with hardware-aware local inference.
