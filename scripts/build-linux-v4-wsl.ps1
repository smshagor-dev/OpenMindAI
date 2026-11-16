param(
  [string]$Distro = "Ubuntu",
  [string]$Version = "4.0.0",
  [ValidateSet("amd64")]
  [string]$Arch = "amd64"
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$releaseOutput = Join-Path $repoRoot "release-output\Linux"
$wslHome = "\\wsl.localhost\$Distro\home\smshagor"
$stageRoot = Join-Path $wslHome "openmindai-builds"
$sourceTar = Join-Path $stageRoot "openmindai-src-$Version.tar"
$distTar = Join-Path $stageRoot "openmindai-dist-$Version.tar"
$wslArtifactDir = "/home/smshagor/openmindai-builds/openmindai-linux-artifacts-$Version"

if (-not (Test-Path $wslHome)) {
  throw "WSL distro '$Distro' is not reachable at $wslHome. Restart WSL/Windows, then rerun this script."
}

New-Item -ItemType Directory -Force -Path $stageRoot, $releaseOutput | Out-Null

Push-Location $repoRoot
try {
  Write-Host "Building frontend on Windows..." -ForegroundColor Cyan
  npm run build

  Write-Host "Staging source archive into WSL..." -ForegroundColor Cyan
  if (Test-Path $sourceTar) {
    Remove-Item -LiteralPath $sourceTar -Force
  }
  git archive --format=tar --output=$sourceTar HEAD

  Write-Host "Staging frontend dist into WSL..." -ForegroundColor Cyan
  if (Test-Path $distTar) {
    Remove-Item -LiteralPath $distTar -Force
  }
  & "$env:SystemRoot\System32\tar.exe" -cf $distTar -C $repoRoot dist
}
finally {
  Pop-Location
}

$bash = @'
set -euo pipefail

export PATH="$HOME/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
export CARGO_HOME="$HOME/openmindai-cargo-home-__VERSION__-__ARCH__"
export CARGO_TARGET_DIR="$HOME/openmindai-cargo-target-__VERSION__-__ARCH__"
export TMPDIR="$HOME/openmindai-tmp-__VERSION__-__ARCH__"
export NPM_CONFIG_CACHE="$HOME/openmindai-npm-cache-__VERSION__-__ARCH__"
export CARGO_PROFILE_RELEASE_LTO=false
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=32
export CARGO_BUILD_JOBS=1
export RUST_BACKTRACE=1

src_tar="$HOME/openmindai-builds/openmindai-src-__VERSION__.tar"
dist_tar="$HOME/openmindai-builds/openmindai-dist-__VERSION__.tar"
workdir="$HOME/openmindai-builds/openmindai-wsl-build-__VERSION__-__ARCH__"
outdir="__WSL_ARTIFACT_DIR__"

rm -rf "$workdir" "$CARGO_HOME" "$CARGO_TARGET_DIR" "$TMPDIR" "$NPM_CONFIG_CACHE"
mkdir -p "$workdir" "$outdir" "$CARGO_HOME" "$CARGO_TARGET_DIR" "$TMPDIR" "$NPM_CONFIG_CACHE"
tar -xf "$src_tar" -C "$workdir"
cd "$workdir"
tar -xf "$dist_tar"

rustup component add rustfmt
npm ci --prefer-online --no-audit

cat > src-tauri/tauri.linux-prebuilt.conf.json <<'JSON'
{
  "$schema": "https://schema.tauri.app/config/2",
  "build": {
    "beforeBuildCommand": "",
    "frontendDist": "../dist"
  },
  "bundle": {
    "targets": ["deb", "appimage"],
    "createUpdaterArtifacts": false
  }
}
JSON

npm run tauri -- build --config src-tauri/tauri.linux-prebuilt.conf.json

find "$CARGO_TARGET_DIR/release/bundle" -type f \( -name "*.deb" -o -name "*.AppImage" \) -print -exec cp {} "$outdir/" \;
cd "$outdir"
tar -czf OpenMindAI___VERSION___linux___ARCH___artifacts.tar.gz OpenMindAI___VERSION_____ARCH__.deb OpenMindAI___VERSION_____ARCH__.AppImage
sha256sum OpenMindAI___VERSION_____ARCH__.deb OpenMindAI___VERSION_____ARCH__.AppImage OpenMindAI___VERSION___linux___ARCH___artifacts.tar.gz > SHA256SUMS-__ARCH__.txt
ls -lh "$outdir"
'@

$bash = $bash.
  Replace("__VERSION__", $Version).
  Replace("__ARCH__", $Arch).
  Replace("__WSL_ARTIFACT_DIR__", $wslArtifactDir)

Write-Host "Building Linux $Arch .deb and .AppImage in WSL..." -ForegroundColor Cyan
$bash -replace "`r", "" | wsl -d $Distro --cd "~" -- bash

$artifactUnc = Join-Path $wslHome "openmindai-builds\openmindai-linux-artifacts-$Version"
$expected = @(
  "OpenMindAI_${Version}_${Arch}.deb",
  "OpenMindAI_${Version}_${Arch}.AppImage",
  "OpenMindAI_${Version}_linux_${Arch}_artifacts.tar.gz",
  "SHA256SUMS-${Arch}.txt"
)

foreach ($name in $expected) {
  $source = Join-Path $artifactUnc $name
  if (-not (Test-Path $source)) {
    throw "Expected artifact was not created: $source"
  }
  Copy-Item -LiteralPath $source -Destination (Join-Path $releaseOutput $name) -Force
}

Write-Host "Linux artifacts copied to $releaseOutput" -ForegroundColor Green
Get-ChildItem -LiteralPath $releaseOutput -File |
  Where-Object { $_.Name -like "OpenMindAI_${Version}_${Arch}*" -or $_.Name -eq "SHA256SUMS-${Arch}.txt" } |
  Select-Object FullName, Length, LastWriteTime
