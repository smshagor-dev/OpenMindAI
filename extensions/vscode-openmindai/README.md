# OpenMindAI for VS Code

Use your local OpenMindAI assistant from VS Code.

This extension talks to the local OpenMindAI API. By default it auto-discovers a running local desktop runtime:

```txt
openmindai.endpoint = auto
```

Current MVP features:

- Native VS Code Chat participant: `@openmindai`
- Right-side Secondary Side Bar tab beside Codex and Claude Code
- Codex-style progress states while planning, reading, thinking, and writing
- Direct workspace edits from chat responses
- Local connection check
- Ask OpenMindAI with active editor context
- Explain selected code
- Propose a selected-code fix with a diff preview before applying

## Production build

From this extension folder:

```powershell
npm ci
npm run compile
npm exec --yes --package=@vscode/vsce@4.0.0 -- vsce package --out dist/openmindai-vscode-0.1.10-marketplace.vsix
```

The upload-ready package is:

```txt
dist/openmindai-vscode-0.1.10-marketplace.vsix
```

## Manual install

Install or upgrade the generated VSIX:

```powershell
code --install-extension dist/openmindai-vscode-0.1.10-marketplace.vsix --force
```

Reload VS Code after installing.

## Setup

1. Start the OpenMindAI desktop app so the local API is running.
2. In VS Code, run `OpenMindAI: Check Local Connection` from the Command Palette.
3. Open the right-side panel with `OpenMindAI: Open Right-Side Chat`.
4. Use VS Code Chat with `@openmindai` for native chat participant workflows.
5. Select code and use `OpenMindAI: Explain Selection` or `OpenMindAI: Fix Selection` from the editor context menu.

Default settings:

```txt
openmindai.endpoint = auto
openmindai.model = openmind-local
openmindai.maxOutputTokens = 2048
openmindai.autoStartNativeInterface = false
```

## Coding agent

With `openmindai.endpoint = auto`, VS Code uses the coding agent configured in the desktop app under **Settings → Agent Setup**: the active OpenAgent model (an NVIDIA Nemotron model such as OpenAgent Lite, `nvidia/NVIDIA-Nemotron-3-Nano-4B-GGUF`) with the Agent Setup context size and GPU layers. There is no separate model setting for this. Change the agent in Agent Setup and VS Code follows it.

The desktop app publishes a local coding-agent endpoint and records it in `%LOCALAPPDATA%\OpenMindAI\coding-agent-endpoint.json`, together with a per-session token that every request must send. The extension only connects to loopback addresses from that file, skips it when the desktop process is gone, and checks the instance id the endpoint reports, so a stale file never leads it to another program on a reused port.

For each request the desktop loads the Agent Setup model if it is not resident, fits the output budget to the model's real context window, and turns off Nemotron's default thinking mode so reasoning does not use up the answer budget. Reasoning text is never shown as the answer. The progress line and `OpenMindAI: Check Local Connection` show which model answered.

The first request after the agent is unloaded can take a few minutes while the model loads from disk. VS Code shows this as progress, for example `OpenMindAI is loading OpenAgent Lite… 45s`, and sends the request once the model is ready. Requests that arrive during the load wait for the same load instead of starting another one. The desktop gives up after 300 seconds and reports why.

In the desktop app, **Settings → Agent Setup → Coding agent** shows the agent state (Disabled, Stopped, Starting, Loading model, Ready or Error), the active model, and the effective context per request. **Agent runtime mode** controls where the agent runs:

- **Automatic** (default): the agent gets its own llama-server next to OpenMindAI Core when both fit in memory, so switching between desktop chat and coding requests does not reload models. On machines where they do not both fit (for example a 4 GB GPU), they share one runtime and the models swap.
- **Shared runtime**: Core and the agent always share one llama-server.
- **Dedicated runtime**: the agent always gets its own llama-server. If the GPU cannot hold both models, the agent runs on the CPU so Core keeps the GPU; if there is not enough RAM either, the shared runtime is used.

If Coding Workspace is disabled in Agent Setup, VS Code shows that coding requests are unavailable instead of answering with another model.

If the desktop app is an older build without this endpoint, the extension falls back to the desktop llama-server directly. That path uses whichever model is loaded, which may not be the Agent Setup coding agent. The extension says so in the progress line, the connection check, and the OpenMindAI output channel.

If the desktop runtime is not running, configure `openmindai.nativeInterfaceRoot` and use `OpenMindAI: Start Native Interface` as a fallback. The native interface only receives standard request fields.

## Output tokens

`openmindai.maxOutputTokens` (default `2048`, range `16`-`8192`) is the response token budget for each request. Raise it for large file edits, because edit responses contain the full new file content. If a reply stops at this limit before any answer is produced, the extension tells you to increase it instead of showing an empty reply.

The budget never exceeds what fits in the model's context window after the prompt. The context that counts is the context per request: Agent Setup splits the configured context across its parallel workers, so 8192 tokens with 2 workers gives each request about 4096. The coding agent and llama-server use the per-slot context the server reports, and Agent Setup shows the same value. The native interface fallback uses `openmindai.nativeContextSize` and also caps the budget at half of it. If the prompt alone does not fit, the request fails with a message instead of being truncated.

## Edit format

To create or change files, the agent replies with one fenced block tagged `openmindai-edits` and nothing else. This is the canonical format:

````txt
```openmindai-edits
{"summary":"what changed","files":[{"path":"relative/path.ext","content":"full file content"}]}
```
````

`content` is the complete new file. Paths must be relative to the first workspace folder. Absolute paths, drive letters, UNC paths, `..` traversal, `.git` internals, and links that point outside the workspace are rejected, and nothing is written unless every file in the plan passes validation.

For compatibility, a reply that is only a JSON object with exactly this schema (optionally inside a single `json` fence) is applied too. JSON that does not match the schema, or JSON mixed with other text, is shown as a normal chat message and never writes files.

## Marketplace upload

Upload `dist/openmindai-vscode-0.1.10-marketplace.vsix` in the Visual Studio Marketplace publisher portal as a Visual Studio Code extension.

## Development

```powershell
npm install
npm run compile
npm test
npm run package
```

`npm test` compiles the extension and runs the Node test suite in `test/`. `npm run package` writes `dist/openmindai-vscode-<version>.vsix`.

`npm run test:e2e` runs `test-e2e/` in an isolated VS Code Extension Development Host (temporary user data and extensions) against the running OpenMindAI desktop app. It drives the Ask, Explain and Fix commands, the right-side view, the editor panel and the `@openmindai` chat participant in a temporary workspace. For the Chat view it loads a test-only language model from `test-e2e/lm-provider/` (the isolated profile has no chat model, and VS Code only dispatches Chat messages when a model is selected), selects it in the Chat model picker, and submits `@openmindai …` through the real Chat input. The test model never answers OpenMindAI requests and is not part of the packaged extension. Set `VSCODE_EXECUTABLE` if VS Code is not in the default per-user location. Setting `OPENMINDAI_SQLITE` and `OPENMINDAI_DB` also checks the Coding Workspace disabled message; the suite restores the setting afterwards.

Then open this folder in VS Code and press `F5` to launch the Extension Development Host.
