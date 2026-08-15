# Portable Storage

During development, use the repository root as `OPENMINDAI_ROOT`, such as `G:\portable ai`.

Portable mode resolves the root from `openmindai.marker` near the executable or one of its parents. The repository root includes this marker so local runtime data stays under the single project root.

Required directories are created automatically at startup:

```text
models/llm/
models/vision/
models/embedding/
models/image/
runtimes/llama/
runtimes/diffusion/
runtimes/python/
runtimes/node/
runtimes/git/
data/database/
data/memory/
data/vectors/
data/indexes/
workspaces/
knowledge/
generated/images/
generated/files/
generated/exports/
cache/
temp/
logs/
config/
backups/
```

Business logic must call `PortableRootManager` for root-owned paths. Hardcoded drive letters, AppData, registry storage, and user-profile storage are not allowed for root-owned *user data* (models, runtimes, database, logs, etc.). This is separate from where the installed application binary itself lives — the production installer places the app in a normal Windows install location (e.g. Program Files), independent of `OPENMINDAI_ROOT`; the root never contains app files, only user data. `install.json` (the small pointer to which root/profile the user chose) lives outside the root too, under `%LOCALAPPDATA%\OpenMindAI`.

Milestone 2 validation uses the repository root as the development root and confirms:

- required directories were created automatically
- SQLite was created at `data/database/openmind_ai.db`
- WAL sidecar files were created beside the database
- restart persistence worked through the backend repository layer

Phase 2.1 stages the llama.cpp runtime entirely under:

```text
G:\portable ai\runtimes\llama\vulkan\<version>
```

Temporary download files are created under `temp/downloads` and removed after extraction. No runtime package is installed to AppData, Downloads, Documents, Program Files, PATH, or any sibling project directory.

## Phase 2 Milestone 2 Storage Audit

The Qwen3 4B GGUF is installed under the single root only:

```text
G:\portable ai\models\llm\qwen\qwen3-4b\Qwen3-4B-Q4_K_M.gguf
```

Download staging used:

```text
G:\portable ai\temp\downloads\Qwen3-4B-Q4_K_M.gguf.part
```

The completed `.part` file was atomically finalized and removed. Runtime logs and validation captures are under `G:\portable ai\logs`. SQLite remains under `G:\portable ai\data\database\openmind_ai.db`.

The model registry stores the installed model path as a portable relative path:

```text
models/llm/qwen/qwen3-4b/Qwen3-4B-Q4_K_M.gguf
```

Runtime-generated folders (`models`, `runtimes`, `data`, `cache`, `temp`, `logs`, and `generated`) remain ignored by Git.
