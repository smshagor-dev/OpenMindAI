from pathlib import Path

path = Path("README.md")
text = path.read_text(encoding="utf-8")
start = text.index("## Current Release\n")
end = text.index("\n## What It Can Do\n", start)
replacement = '''## Current Release

**Current source version: v3.0.0 (release candidate)**

**Latest public release: [v2.0.0](https://github.com/smshagor-dev/OpenMindAI/releases/tag/v2.0.0)**

The v3.0.0 source candidate turns Projects into an active local development workspace: a folder can be opened as a project, the local agent can inspect and edit files, run explicitly permitted terminal commands, observe failures, retry repairs, validate changes, and inspect Git state while preserving the Full PC + Terminal permission boundary.

The public download table stays on v2.0.0 until the signed v3.0.0 release pipeline has produced and verified the installer, updater signature, metadata, checksums, clean install, and v2-to-v3 upgrade path.

| Platform | Installation path | Current status |
| --- | --- | --- |
| Windows x64 | [OpenMindAI v2.0.0 installer](https://github.com/smshagor-dev/OpenMindAI/releases/download/v2.0.0/OpenMindAI_2.0.0_x64-setup.exe) | Latest public Windows release |
| Windows | [Git bootstrap](https://github.com/smshagor-dev/OpenMindAI/releases/download/v2.0.0/OpenMindAI-Setup.bat) | Available |
| Linux x64 | [Shell bootstrap](https://github.com/smshagor-dev/OpenMindAI/releases/download/v2.0.0/openmindai-setup.sh) | Implemented; broader hardware validation in progress |
| macOS | [Command bootstrap](https://github.com/smshagor-dev/OpenMindAI/releases/download/v2.0.0/OpenMindAI-Setup.command) | Implemented; broader hardware validation in progress |
| Linux ARM64 | — | Not currently supported |

For release notes and downloadable assets, use the [Releases](https://github.com/smshagor-dev/OpenMindAI/releases) page rather than third-party mirrors.

> Production v3.0.0 remains intentionally unpublished until the signing and installer trust gates pass. The release workflow fails closed when signing material is unavailable.
'''
path.write_text(text[:start] + replacement + text[end:], encoding="utf-8")
print("README release state corrected for the v3.0.0 release candidate.")
