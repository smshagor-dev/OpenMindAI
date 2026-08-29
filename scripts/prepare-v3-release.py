from pathlib import Path
import json
import re

VERSION = "3.0.0"
TAG = f"v{VERSION}"


def read(path: str) -> str:
    return Path(path).read_text(encoding="utf-8")


def write(path: str, content: str) -> None:
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(content, encoding="utf-8")


def write_json(path: str, data: object) -> None:
    write(path, json.dumps(data, indent=2) + "\n")


package = json.loads(read("package.json"))
package["version"] = VERSION
write_json("package.json", package)

lock = json.loads(read("package-lock.json"))
lock["version"] = VERSION
lock.setdefault("packages", {}).setdefault("", {})["version"] = VERSION
write_json("package-lock.json", lock)

tauri = json.loads(read("src-tauri/tauri.conf.json"))
tauri["version"] = VERSION
write_json("src-tauri/tauri.conf.json", tauri)

cargo_toml = read("src-tauri/Cargo.toml")
cargo_toml, count = re.subn(
    r'(?ms)(\[package\].*?^version\s*=\s*")[^"]+("\s*$)',
    rf'\g<1>{VERSION}\g<2>',
    cargo_toml,
    count=1,
)
if count != 1:
    raise SystemExit("failed to update src-tauri/Cargo.toml package version")
write("src-tauri/Cargo.toml", cargo_toml)

cargo_lock = read("src-tauri/Cargo.lock")
cargo_lock, count = re.subn(
    r'(\[\[package\]\]\nname = "open-mind-ai"\nversion = ")[^"]+("\n)',
    rf'\g<1>{VERSION}\g<2>',
    cargo_lock,
    count=1,
)
if count != 1:
    raise SystemExit("failed to update root package version in src-tauri/Cargo.lock")
write("src-tauri/Cargo.lock", cargo_lock)

marker = read("openmindai.marker")
marker, count = re.subn(r'(?m)^version=.*$', f"version={VERSION}", marker, count=1)
if count != 1:
    raise SystemExit("failed to update openmindai.marker")
write("openmindai.marker", marker)

for launcher in ("OpenMindAI-Setup.bat", "OpenMindAI-Setup.command", "openmindai-setup.sh"):
    text = read(launcher)
    text, count = re.subn(
        r'(?m)^(rem |# )Version: [^\r\n]+$',
        rf'\g<1>Version: {VERSION}',
        text,
        count=1,
    )
    if count != 1:
        raise SystemExit(f"failed to update launcher version: {launcher}")
    write(launcher, text)

readme = read("README.md")
readme = readme.replace("v2.0.0", TAG).replace("2.0.0", VERSION)
release_sentence = (
    f"The {TAG} release includes the Windows installer together with bootstrap installers for Windows, Linux, and macOS."
)
expanded_release = release_sentence + "\n\n" + (
    "Version 3 turns Projects into an active local development workspace: a folder can be opened as a project, "
    "the local agent can inspect and edit files, run permitted terminal commands, observe failures, retry repairs, "
    "validate changes, and inspect Git state while preserving the explicit Full PC + Terminal permission boundary."
)
if release_sentence in readme:
    readme = readme.replace(release_sentence, expanded_release, 1)
else:
    raise SystemExit("README current release paragraph anchor not found")

project_old = (
    "Projects can group conversations, instructions, and local files. Text and code files can become project context, "
    "while larger or binary files remain tracked as local project resources."
)
project_new = (
    "Projects can group conversations, instructions, and local files. A local folder can also be opened directly as a project. "
    "Project Agent uses the attached workspace as live context and can read, create, edit, rename, or delete scoped files. "
    "When the user explicitly enables Full PC + Terminal access, the agent can run non-interactive shell commands and hardened "
    "Git status/diff inspection, recover from non-zero command failures, refresh its workspace snapshot after edits, and require "
    "appropriate validation before reporting a changed workspace as complete. Text and code files can become project context, "
    "while larger or binary files remain tracked as local project resources."
)
if project_old not in readme:
    raise SystemExit("README Projects paragraph anchor not found")
readme = readme.replace(project_old, project_new, 1)
write("README.md", readme)

changelog = f'''# Changelog

All notable OpenMindAI release changes are documented here.

## [{VERSION}] - 2026-08-29

### Added

- One-click **Open Folder as Project** flow that creates a project, attaches the selected local folder, creates the linked chat, and rolls back partial setup if the durable project-chat link is not completed.
- Local **Project Agent** execution loop for workspace inspection, file mutation, terminal-assisted development, Git inspection, and iterative repair/validation.
- Structured `git_status` and `git_diff` agent tools with external diff/textconv helpers and Git fsmonitor disabled for safer inspection.
- Release-readiness documentation and a dedicated v3 release contract covering synchronized versions, updater metadata, installer assets, and bootstrap launchers.

### Changed

- Project Agent now keeps recent project-chat context, uses the llama-compatible `/v1/chat/completions` endpoint, and retries transient model-unavailable responses.
- Autonomous task budget increased to support realistic inspect → edit → test → recover workflows while retaining a hard upper bound.
- Workspace context refreshes after successful file mutations so later reasoning uses the current project state.
- Terminal execution supports bounded per-command timeouts and treats timeout/non-zero exits as failed observations rather than successful tool calls.
- Agent failure accounting is consecutive instead of lifetime-based, and repeated identical actions are blocked to prevent unproductive loops.
- Host-enforced validation prevents the agent from finalizing modified workspaces without an applicable successful test/build/lint/check, unless it explicitly records why automated validation is not meaningful.
- Terminal planning is host-aware: Windows uses non-interactive Windows PowerShell; macOS/Linux use `/bin/sh -lc`.

### Security

- Full PC, terminal, and process-based Git inspection remain behind the explicit **Full PC + Terminal** grant.
- Production release pipeline requires Tauri updater signing material plus a Windows Authenticode certificate and verifies the resulting installer signature, updater signature contract, HTTPS release URL, and SHA-256 checksums before a release candidate can be trusted.
- CI and Security continue to enforce frontend validation, Rust formatting, strict Clippy, cross-platform Rust tests, Windows desktop build, dependency audits, and CodeQL.

### Release validation

- Windows release candidate is required to pass a silent clean-install smoke test.
- The production release pipeline also verifies an in-place installer upgrade path from the public v2.0.0 Windows installer to v3.0.0.
- Bootstrap assets for Windows, macOS, and Linux plus `openmindai.marker` are included in the verified release asset contract.

## [2.0.0] - 2026-08-21

- Major local-first desktop release with model management, internal routing, Projects, Tools, media entry points, setup improvements, and the Windows installer.
'''
write("CHANGELOG.md", changelog)

release_notes = f'''# OpenMindAI {TAG}

OpenMindAI {TAG} makes local Projects actionable. The desktop app can now attach a real folder to a project and route project chat through a local agent that can inspect the workspace, make scoped file changes, use permitted terminal commands, observe failures, repair them, validate the result, and report what changed.

## Highlights

### Open Folder as Project

- Select an existing folder and create/attach an OpenMindAI Project in one flow.
- The linked project chat is created automatically.
- Partial setup rolls back orphan project/chat state if the durable link is not completed.
- Attached-folder filesystem access remains scoped unless Full PC + Terminal access is explicitly approved.

### Local Project Agent

- Reads and edits project files locally.
- Creates, renames, and deletes workspace files when required by the task.
- Uses live workspace snapshots and refreshes context after successful edits.
- Uses recent project conversation context instead of treating each project instruction as an isolated turn.
- Supports responsive cancellation while waiting on the model or a tool.

### Plan, execute, validate, recover

- Non-zero terminal exits and timeouts are surfaced as failures for the next reasoning step.
- Successful recovery resets the consecutive-failure budget.
- Duplicate action protection prevents the agent from repeating the same broken action indefinitely.
- Modified workspaces must pass an applicable validation command before completion unless the agent records a concrete reason why automated validation does not apply.
- Validation commands are credited only when their exit status is authoritative; chained commands cannot mask an earlier failure.

### Git-aware local work

- Structured Git status and diff inspection are available to the agent when Full PC + Terminal access is enabled.
- Git inspection disables fsmonitor, external diff, textconv, and recursive submodule behavior that could unexpectedly execute external helpers.
- Commits, branch changes, resets, cleans, installs, and other arbitrary terminal operations remain governed by the terminal permission boundary and the user's instruction.

### Cross-platform execution

- Windows agent terminal guidance targets non-interactive Windows PowerShell.
- macOS and Linux guidance targets `/bin/sh -lc`.
- The agent is instructed to avoid interactive prompts, editors, pagers, sudo/password requests, or commands that wait indefinitely for user input.

## Validation and security

The v3 source release is gated by:

- frontend lint, TypeScript, and Vite production build;
- Rust formatting with `rustfmt`;
- strict Clippy with warnings denied;
- Rust tests on Windows, macOS, and Ubuntu;
- Windows Tauri application build and executable verification;
- `npm audit` and `cargo audit`;
- CodeQL analysis;
- updater/release configuration checks;
- Windows NSIS clean-install and v2.0.0 → v3.0.0 upgrade smoke validation in the release path;
- Authenticode and Tauri updater signing checks for the production tagged release.

## Release assets

A trusted Windows production release is expected to include:

- `OpenMindAI_{VERSION}_x64-setup.exe`
- `OpenMindAI_{VERSION}_x64-setup.exe.sig`
- `latest.json`
- `SHA256SUMS.txt`
- `OpenMindAI-Setup.bat`
- `OpenMindAI-Setup.command`
- `openmindai-setup.sh`
- `openmindai.marker`

Model weights, runtimes, local databases, user projects, logs, caches, and other machine-local data are not included in the GitHub source repository or release installer.

## Upgrade note

OpenMindAI keeps the user's selected AI data root separate from the desktop application. Updating the application must not be treated as permission to delete model files, chat history, projects, or other data stored in the configured OpenMindAI root.
'''
write(f"docs/releases/{TAG}.md", release_notes)

version_script = r'''import fs from "node:fs";
import path from "node:path";

const root = process.cwd();

function readText(relativePath) {
  return fs.readFileSync(path.join(root, relativePath), "utf8");
}

function readJson(relativePath) {
  return JSON.parse(readText(relativePath));
}

function fail(message) {
  throw new Error(`Version consistency check failed: ${message}`);
}

function readCargoPackageVersion() {
  const cargoToml = readText(path.join("src-tauri", "Cargo.toml"));
  let inPackageSection = false;
  for (const rawLine of cargoToml.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (/^\[[^\]]+\]$/.test(line)) {
      inPackageSection = line === "[package]";
      continue;
    }
    if (inPackageSection) {
      const match = line.match(/^version\s*=\s*"([^"]+)"\s*$/);
      if (match) return match[1];
    }
  }
  fail("unable to locate package version in src-tauri/Cargo.toml");
}

function readCargoLockVersion() {
  const cargoLock = readText(path.join("src-tauri", "Cargo.lock"));
  const block = cargoLock
    .split("[[package]]")
    .find((candidate) => /(?:^|\n)name = "open-mind-ai"(?:\n|$)/.test(candidate));
  const match = block?.match(/(?:^|\n)version = "([^"]+)"(?:\n|$)/);
  if (!match) fail("unable to locate open-mind-ai version in src-tauri/Cargo.lock");
  return match[1];
}

function readMarkerVersion() {
  const match = readText("openmindai.marker").match(/^version=(.+)$/m);
  if (!match) fail("openmindai.marker does not contain version=");
  return match[1].trim();
}

function readLauncherVersion(relativePath) {
  const match = readText(relativePath).match(/^(?:rem |# )Version:\s*([^\r\n]+)$/m);
  if (!match) fail(`${relativePath} does not contain a Version header`);
  return match[1].trim();
}

function readRequestedTag() {
  const index = process.argv.indexOf("--tag");
  if (index === -1) return null;
  const value = process.argv[index + 1];
  if (!value || value.startsWith("--")) fail("--tag requires a value such as v3.0.0");
  return value;
}

const packageJson = readJson("package.json");
const packageLock = readJson("package-lock.json");
const tauriConfig = readJson(path.join("src-tauri", "tauri.conf.json"));
const versions = new Map([
  ["package.json", packageJson.version],
  ["package-lock.json", packageLock.version],
  ["package-lock.json packages['']", packageLock.packages?.[""]?.version],
  ["src-tauri/tauri.conf.json", tauriConfig.version],
  ["src-tauri/Cargo.toml", readCargoPackageVersion()],
  ["src-tauri/Cargo.lock", readCargoLockVersion()],
  ["openmindai.marker", readMarkerVersion()],
  ["OpenMindAI-Setup.bat", readLauncherVersion("OpenMindAI-Setup.bat")],
  ["OpenMindAI-Setup.command", readLauncherVersion("OpenMindAI-Setup.command")],
  ["openmindai-setup.sh", readLauncherVersion("openmindai-setup.sh")],
]);

for (const [source, version] of versions) {
  if (typeof version !== "string" || !/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) {
    fail(`${source} does not contain a valid semantic version: ${String(version)}`);
  }
}

const canonicalVersion = packageJson.version;
const mismatches = [...versions.entries()].filter(([, version]) => version !== canonicalVersion);
if (mismatches.length > 0) {
  fail(`version mismatch: package.json=${canonicalVersion}; ${mismatches.map(([source, version]) => `${source}=${version}`).join(", ")}`);
}

const expectedTag = `v${canonicalVersion}`;
const releaseNotesPath = path.join(root, "docs", "releases", `${expectedTag}.md`);
if (!fs.existsSync(releaseNotesPath)) fail(`missing release notes: docs/releases/${expectedTag}.md`);
const releaseNotes = fs.readFileSync(releaseNotesPath, "utf8");
if (!releaseNotes.includes(`# OpenMindAI ${expectedTag}`)) fail("release notes title does not match synchronized version");
if (/\b(?:TODO|TBD|PLACEHOLDER)\b/i.test(releaseNotes)) fail("release notes contain unfinished placeholder text");

const changelog = readText("CHANGELOG.md");
if (!changelog.includes(`## [${canonicalVersion}]`)) fail("CHANGELOG.md is missing the current version section");

const readme = readText("README.md");
if (!readme.includes(`Latest public release: [${expectedTag}]`)) fail("README.md current release does not match synchronized version");
if (!readme.includes(`OpenMindAI_${canonicalVersion}_x64-setup.exe`)) fail("README.md Windows installer link does not match synchronized version");

const requestedTag = readRequestedTag();
if (requestedTag !== null && requestedTag !== expectedTag) {
  fail(`release tag ${requestedTag} does not match synchronized app version ${expectedTag}`);
}

console.log(`Version consistency OK: ${canonicalVersion}${requestedTag ? ` (${requestedTag})` : ""}; manifests, locks, marker, launchers, README, changelog, and release notes are synchronized.`);
'''
write("scripts/check-version-consistency.mjs", version_script)

release_check = read("scripts/check-release-readiness.mjs")
anchor = "console.log(\n  `Release readiness config OK:"
if anchor not in release_check:
    raise SystemExit("release readiness script output anchor not found")
extra = r'''const releasePackage = readJson("package.json");
const releaseVersion = releasePackage.version;
const releaseTag = `v${releaseVersion}`;
const releaseNotesPath = path.join(root, "docs", "releases", `${releaseTag}.md`);
if (!fs.existsSync(releaseNotesPath)) {
  fail(`missing production release notes: docs/releases/${releaseTag}.md`);
}
const releaseNotes = fs.readFileSync(releaseNotesPath, "utf8");
if (!releaseNotes.includes(`# OpenMindAI ${releaseTag}`)) {
  fail(`release notes title must be '# OpenMindAI ${releaseTag}'`);
}
if (/\b(?:TODO|TBD|PLACEHOLDER)\b/i.test(releaseNotes)) {
  fail("release notes contain unfinished placeholder text");
}
const releaseWorkflow = fs.readFileSync(path.join(root, ".github", "workflows", "release.yml"), "utf8");
for (const requiredSecret of [
  "TAURI_SIGNING_PRIVATE_KEY",
  "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
  "WINDOWS_CERTIFICATE",
  "WINDOWS_CERTIFICATE_PASSWORD",
]) {
  if (!releaseWorkflow.includes(`secrets.${requiredSecret}`)) {
    fail(`production release workflow no longer references required secret ${requiredSecret}`);
  }
}
if (!releaseWorkflow.includes("releaseDraft: true")) {
  fail("production release must remain a draft until signed asset and install/upgrade verification completes");
}
for (const requiredAsset of [
  "OpenMindAI-Setup.bat",
  "OpenMindAI-Setup.command",
  "openmindai-setup.sh",
  "openmindai.marker",
  "latest.json",
  "SHA256SUMS.txt",
]) {
  if (!releaseWorkflow.includes(requiredAsset)) {
    fail(`production release workflow does not enforce required asset ${requiredAsset}`);
  }
}

'''
release_check = release_check.replace(anchor, extra + anchor, 1)
write("scripts/check-release-readiness.mjs", release_check)

release_workflow = read(".github/workflows/release.yml")
download_anchor = "          gh release download $tag --dir $assetDir --clobber\n\n          $installers ="
if download_anchor not in release_workflow:
    raise SystemExit("release workflow download anchor not found")
download_replacement = '''          $releaseNotesPath = "docs/releases/$tag.md"
          if (-not (Test-Path -LiteralPath $releaseNotesPath)) {
            throw "Release notes are missing: $releaseNotesPath"
          }
          gh release edit $tag --notes-file $releaseNotesPath
          foreach ($bootstrapAsset in @("OpenMindAI-Setup.bat", "OpenMindAI-Setup.command", "openmindai-setup.sh", "openmindai.marker")) {
            if (-not (Test-Path -LiteralPath $bootstrapAsset)) {
              throw "Required bootstrap release asset is missing from the source tree: $bootstrapAsset"
            }
            gh release upload $tag $bootstrapAsset --clobber
          }
          gh release download $tag --dir $assetDir --clobber

          $installers ='''
release_workflow = release_workflow.replace(download_anchor, download_replacement, 1)

smoke_anchor = '''          if ($urlAssetName -ne $installer.Name) {
            throw "latest.json points to $urlAssetName but the signed installer is $($installer.Name)."
          }

          $checksumPath ='''
if smoke_anchor not in release_workflow:
    raise SystemExit("release workflow smoke-test anchor not found")
smoke_block = '''          if ($urlAssetName -ne $installer.Name) {
            throw "latest.json points to $urlAssetName but the signed installer is $($installer.Name)."
          }

          function Invoke-SilentInstaller {
            param([string]$InstallerPath, [string]$Destination)
            if (Test-Path -LiteralPath $Destination) { Remove-Item -LiteralPath $Destination -Recurse -Force }
            New-Item -ItemType Directory -Path $Destination -Force | Out-Null
            $process = Start-Process -FilePath $InstallerPath -ArgumentList @('/S', "/D=$Destination") -Wait -PassThru
            if ($process.ExitCode -ne 0) {
              throw "Installer failed with exit code $($process.ExitCode): $InstallerPath"
            }
          }

          function Resolve-InstalledApp {
            param([string]$Destination)
            $candidate = Get-ChildItem -LiteralPath $Destination -Filter '*.exe' -File -Recurse |
              Where-Object { $_.Name -notmatch '(?i)uninstall' } |
              Sort-Object Length -Descending |
              Select-Object -First 1
            if ($null -eq $candidate) {
              throw "No installed application executable found under $Destination"
            }
            return $candidate
          }

          function Assert-InstalledVersion {
            param([string]$Destination, [string]$ExpectedVersion)
            $appExe = Resolve-InstalledApp -Destination $Destination
            $productVersion = $appExe.VersionInfo.ProductVersion
            $fileVersion = $appExe.VersionInfo.FileVersion
            if (($productVersion -notlike "$ExpectedVersion*") -and ($fileVersion -notlike "$ExpectedVersion*")) {
              throw "Installed executable version mismatch. Expected $ExpectedVersion; product=$productVersion file=$fileVersion path=$($appExe.FullName)"
            }
          }

          function Remove-SmokeInstall {
            param([string]$Destination)
            $uninstaller = Get-ChildItem -LiteralPath $Destination -Filter '*.exe' -File -Recurse |
              Where-Object { $_.Name -match '(?i)uninstall' } |
              Select-Object -First 1
            if ($null -ne $uninstaller) {
              $process = Start-Process -FilePath $uninstaller.FullName -ArgumentList '/S' -Wait -PassThru
              if ($process.ExitCode -ne 0) {
                throw "Uninstaller failed with exit code $($process.ExitCode): $($uninstaller.FullName)"
              }
            }
            if (Test-Path -LiteralPath $Destination) { Remove-Item -LiteralPath $Destination -Recurse -Force }
          }

          $v2InstallerPath = Join-Path $env:RUNNER_TEMP 'OpenMindAI_2.0.0_x64-setup.exe'
          Invoke-WebRequest -UseBasicParsing -Uri 'https://github.com/smshagor-dev/OpenMindAI/releases/download/v2.0.0/OpenMindAI_2.0.0_x64-setup.exe' -OutFile $v2InstallerPath

          $upgradeRoot = Join-Path $env:RUNNER_TEMP 'OpenMindAI-v3-upgrade-smoke'
          Invoke-SilentInstaller -InstallerPath $v2InstallerPath -Destination $upgradeRoot
          $upgradeProcess = Start-Process -FilePath $installer.FullName -ArgumentList @('/S', "/D=$upgradeRoot") -Wait -PassThru
          if ($upgradeProcess.ExitCode -ne 0) {
            throw "v2.0.0 to $expectedVersion upgrade installer failed with exit code $($upgradeProcess.ExitCode)."
          }
          Assert-InstalledVersion -Destination $upgradeRoot -ExpectedVersion $expectedVersion
          Remove-SmokeInstall -Destination $upgradeRoot

          $cleanRoot = Join-Path $env:RUNNER_TEMP 'OpenMindAI-v3-clean-smoke'
          Invoke-SilentInstaller -InstallerPath $installer.FullName -Destination $cleanRoot
          Assert-InstalledVersion -Destination $cleanRoot -ExpectedVersion $expectedVersion
          Remove-SmokeInstall -Destination $cleanRoot
          Write-Host "Signed Windows installer clean-install and v2.0.0 -> $expectedVersion upgrade smoke tests passed."

          $checksumPath ='''
release_workflow = release_workflow.replace(smoke_anchor, smoke_block, 1)

old_count = '''          if ($lines.Count -lt 3) {
            throw "Release asset set is incomplete; expected installer, signature, and latest.json."
          }'''
new_count = '''          if ($lines.Count -lt 7) {
            throw "Release asset set is incomplete; expected installer, signature, latest.json, and four bootstrap assets."
          }'''
if old_count not in release_workflow:
    raise SystemExit("release workflow checksum-count anchor not found")
release_workflow = release_workflow.replace(old_count, new_count, 1)

required_old = '          foreach ($required in @($installer.Name, "$($installer.Name).sig", "latest.json", "SHA256SUMS.txt")) {'
required_new = '          foreach ($required in @($installer.Name, "$($installer.Name).sig", "latest.json", "SHA256SUMS.txt", "OpenMindAI-Setup.bat", "OpenMindAI-Setup.command", "openmindai-setup.sh", "openmindai.marker")) {'
if required_old not in release_workflow:
    raise SystemExit("release workflow required-assets anchor not found")
release_workflow = release_workflow.replace(required_old, required_new, 1)
write(".github/workflows/release.yml", release_workflow)

print("OpenMindAI v3.0.0 release preparation staged successfully.")
