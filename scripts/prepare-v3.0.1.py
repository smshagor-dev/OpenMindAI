from pathlib import Path
import json
import re

VERSION = "3.0.1"
TAG = f"v{VERSION}"
PREVIOUS_VERSION = "3.0.0"
PREVIOUS_TAG = f"v{PREVIOUS_VERSION}"


def read(path: str) -> str:
    return Path(path).read_text(encoding="utf-8")


def write(path: str, content: str) -> None:
    Path(path).write_text(content, encoding="utf-8")


# Node manifests / lockfile
package_path = Path("package.json")
package = json.loads(package_path.read_text(encoding="utf-8"))
package["version"] = VERSION
package_path.write_text(json.dumps(package, indent=2) + "\n", encoding="utf-8")

lock_path = Path("package-lock.json")
lock = json.loads(lock_path.read_text(encoding="utf-8"))
lock["version"] = VERSION
lock.setdefault("packages", {}).setdefault("", {})["version"] = VERSION
lock_path.write_text(json.dumps(lock, indent=2) + "\n", encoding="utf-8")

# Tauri config
tauri_path = Path("src-tauri/tauri.conf.json")
tauri = json.loads(tauri_path.read_text(encoding="utf-8"))
tauri["version"] = VERSION
tauri_path.write_text(json.dumps(tauri, indent=2) + "\n", encoding="utf-8")

# Cargo package manifest
cargo_toml = read("src-tauri/Cargo.toml")
cargo_toml, count = re.subn(
    r'(?ms)(^\[package\]\s*.*?^version\s*=\s*")([^"]+)(")',
    rf'\g<1>{VERSION}\g<3>',
    cargo_toml,
    count=1,
)
if count != 1:
    raise SystemExit("Unable to update src-tauri/Cargo.toml package version")
write("src-tauri/Cargo.toml", cargo_toml)

# Cargo.lock: update only open-mind-ai package block
cargo_lock = read("src-tauri/Cargo.lock")
pattern = r'(?ms)(\[\[package\]\]\nname = "open-mind-ai"\nversion = ")([^"]+)(")'
cargo_lock, count = re.subn(pattern, rf'\g<1>{VERSION}\g<3>', cargo_lock, count=1)
if count != 1:
    raise SystemExit("Unable to update open-mind-ai entry in Cargo.lock")
write("src-tauri/Cargo.lock", cargo_lock)

# Marker and bootstrap launchers
marker = read("openmindai.marker")
marker, count = re.subn(r'(?m)^version=.*$', f'version={VERSION}', marker, count=1)
if count != 1:
    raise SystemExit("Unable to update openmindai.marker")
write("openmindai.marker", marker)

for launcher in ("OpenMindAI-Setup.bat", "OpenMindAI-Setup.command", "openmindai-setup.sh"):
    text = read(launcher)
    text, count = re.subn(r'(?m)^((?:rem |# )Version:\s*)[^\r\n]+$', rf'\g<1>{VERSION}', text, count=1)
    if count != 1:
        raise SystemExit(f"Unable to update version header in {launcher}")
    write(launcher, text)

# Local installer smoke defaults
smoke = read("scripts/smoke-local-installer.ps1")
smoke = smoke.replace("OpenMindAI_3.0.0_x64-setup.exe", "OpenMindAI_3.0.1_x64-setup.exe")
smoke = smoke.replace("OpenMindAI-v3-local-smoke", "OpenMindAI-v3.0.1-local-smoke")
write("scripts/smoke-local-installer.ps1", smoke)

# Keep version-check examples aligned with the target release.
checker = read("scripts/check-version-consistency.mjs")
checker = checker.replace("such as v3.0.0", "such as v3.0.1")
write("scripts/check-version-consistency.mjs", checker)

# README release section
readme = read("README.md")
release_section = f'''## OpenMindAI {TAG}

**Current source version:** `{TAG}`  
**Release target:** [{TAG}](https://github.com/smshagor-dev/OpenMindAI/releases/tag/{TAG})  
**Previous stable release:** [{PREVIOUS_TAG}](https://github.com/smshagor-dev/OpenMindAI/releases/tag/{PREVIOUS_TAG})

{TAG} is the maintenance release for the current OpenMindAI 3.x desktop line. It keeps the existing local-first architecture while consolidating the latest validated fixes, native runtime work, model routing improvements, and release-pipeline hardening on top of {PREVIOUS_TAG}.

### Download

| Platform | Installation path | Status |
| --- | --- | --- |
| Windows x64 | [OpenMindAI {TAG} installer](https://github.com/smshagor-dev/OpenMindAI/releases/download/{TAG}/OpenMindAI_{VERSION}_x64-setup.exe) | Primary packaged release |
| Windows | [Bootstrap setup](https://github.com/smshagor-dev/OpenMindAI/releases/download/{TAG}/OpenMindAI-Setup.bat) | Release asset |
| Linux x64 | [Shell bootstrap](https://github.com/smshagor-dev/OpenMindAI/releases/download/{TAG}/openmindai-setup.sh) | Release asset; hardware coverage continues to expand |
| macOS | [Command bootstrap](https://github.com/smshagor-dev/OpenMindAI/releases/download/{TAG}/OpenMindAI-Setup.command) | Release asset; hardware coverage continues to expand |
| Linux ARM64 | — | Not currently supported |

Use the official [Releases](https://github.com/smshagor-dev/OpenMindAI/releases) page for signed artifacts, checksums, updater metadata, and release notes.

### What changed in {TAG}

- Synchronizes the desktop application, Tauri bundle, Rust crate, lockfiles, bootstrap launchers, and portable marker on version `{VERSION}`.
- Preserves the local-first desktop architecture and current native model/runtime behavior from the validated `main` branch.
- Carries forward the native personalization and runtime integration work merged after {PREVIOUS_TAG}.
- Keeps CI, CodeQL, dependency auditing, multi-platform Rust validation, Windows Tauri build validation, and release-contract checks as release gates.
- Adds explicit {PREVIOUS_TAG} → {TAG} upgrade validation in addition to clean-install validation.
- Produces release-specific notes and installer naming for `{TAG}` while retaining {PREVIOUS_TAG} as the upgrade source.

For the focused release summary, see [`.github/releases/{TAG}.txt`](.github/releases/{TAG}.txt).

'''
readme, count = re.subn(r'(?ms)^## OpenMindAI v3\.0\.0\n.*?(?=^## Why OpenMindAI\n)', release_section, readme, count=1)
if count != 1:
    raise SystemExit("Unable to replace README release section")
write("README.md", readme)

# Release notes
release_notes = f'''# OpenMindAI {TAG}

OpenMindAI {TAG} is a maintenance release for the 3.x desktop line. It packages the current validated `main` branch as a synchronized release and keeps {PREVIOUS_TAG} as the supported previous-release baseline for upgrade testing.

## Release scope

- Version metadata synchronized to `{VERSION}` across Node, Tauri, Rust, lockfiles, portable marker, and platform bootstrap launchers.
- Windows NSIS installer and updater metadata produced under the `{TAG}` release contract.
- Clean-install validation retained for the native Windows package.
- Upgrade validation targets `{PREVIOUS_TAG}` → `{TAG}` so existing 3.0.0 installations can be tested before publication.
- Native runtime ABI checks, application version checks, updater signature validation, checksums, and release-asset verification remain release gates.
- CI, Rust formatting, Clippy, Rust tests, frontend validation, CodeQL, and dependency audits remain required before release.

## Upgrade from {PREVIOUS_TAG}

Users on {PREVIOUS_TAG} should be able to install {TAG} over the existing installation. The release validation path verifies that the resulting executable reports `{VERSION}` and that the packaged native runtime remains present and ABI-aligned.

## Release assets

The production release contract requires:

- `OpenMindAI_{VERSION}_x64-setup.exe`
- `OpenMindAI_{VERSION}_x64-setup.exe.sig`
- `latest.json`
- `SHA256SUMS.txt`
- `OpenMindAI-Setup.bat`
- `OpenMindAI-Setup.command`
- `openmindai-setup.sh`
- `openmindai.marker`

The GitHub release should remain a draft until signing, updater metadata, checksums, clean installation, and {PREVIOUS_TAG} → {TAG} upgrade validation have passed.
'''
Path(f".github/releases/{TAG}.txt").write_text(release_notes, encoding="utf-8")

print(f"Prepared OpenMindAI {TAG} release metadata from {PREVIOUS_TAG}")
