# OpenMindAI Open Model Catalog

OpenMindAI exposes product-facing model names while retaining upstream repository, license, runtime, and artifact metadata internally for reproducible downloads and attribution.

## Added in catalog version 7

| OpenMindAI name | Internal upstream family | Local package | Primary use |
| --- | --- | --- | --- |
| OpenMindAI Forge | gpt-oss 20B | MXFP4 GGUF | reasoning, coding, tools |
| OpenMindAI Forge Max | gpt-oss 120B | MXFP4 GGUF | high-end reasoning and agents |
| OpenMindAI Flash | Gemma 4 E2B | Q4_0 GGUF + mmproj | fast multimodal work |
| OpenMindAI Flash Plus | Gemma 4 E4B | Q4_0 GGUF + mmproj | stronger multimodal work |
| OpenMindAI Vision | Gemma 4 12B | Q4_0 GGUF + mmproj | balanced visual reasoning |
| OpenMindAI Vision Pro | Gemma 4 26B-A4B | Q4_0 GGUF + mmproj | advanced visual reasoning |
| OpenMindAI Vision Max | Gemma 4 31B | Q4_0 GGUF + mmproj | high-end visual reasoning |
| OpenMindAI Agent Lite | Nemotron 3 Nano 4B | Q4_K_M GGUF | lightweight agent tasks |
| OpenMindAI Agent | Nemotron 3 Nano 30B-A3B | Q4_K_M GGUF | agentic workflows |
| OpenMindAI Agent Lightning | Nemotron 3.5 Lightning 30B-A3B | Q4_0 GGUF | long-running agents |
| OpenMindAI Agent Pro | Nemotron 3 Super 120B | Q4_K GGUF | high-end agent workflows |

The application UI uses only the OpenMindAI names. Upstream identifiers remain internal metadata so model downloads can be verified and licenses can be preserved.

## Scope

The catalog includes current general-purpose open/open-weight models from the requested families that have practical llama.cpp GGUF packages. API-only models, safeguard/classifier-only checkpoints, speculative-decoding sidecars, and impractically large research checkpoints are not exposed as standalone user models.

Gemma 4 multimodal packages include the required `mmproj` artifact. Optional MTP/speculative-decoding files are not downloaded because the current OpenMindAI runtime does not require them.
