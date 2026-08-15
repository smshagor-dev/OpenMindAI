#Requires -Version 5.1
<#
  OpenMindAI Windows bootstrap. Invoked by OpenMindAI-Setup.bat.

  Lifecycle: locate/clone source -> resolve a prebuilt release for this
  OS/arch (preferred) -> fall back to building from source (developer mode
  only) -> launch OpenMindAI.

  Safe to run repeatedly: an already-installed OpenMindAI is detected and
  launched directly without re-cloning, re-installing dependencies, or
  rebuilding. Never destroys local modifications in an existing source
  checkout (fetch + fast-forward only -- see Sync-Source).
#>
param(
  # Directory OpenMindAI-Setup.bat was launched from. Falls back to this
  # script's own grandparent directory if invoked directly (e.g. while
  # developing this script itself).
  [string]$LauncherRoot,
  [switch]$DeveloperMode,
  [switch]$NoLaunch
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Write-Step($message) { Write-Host "==> $message" -ForegroundColor Cyan }
function Write-Info($message) { Write-Host "    $message" -ForegroundColor Gray }
function Write-Warn($message) { Write-Host "    $message" -ForegroundColor Yellow }
function Write-Err($message) { Write-Host "[OpenMindAI Setup] $message" -ForegroundColor Red }

$ScriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $LauncherRoot) {
  $LauncherRoot = Split-Path -Parent (Split-Path -Parent $ScriptRoot)
}
# Defensive: a caller passing a path ending in "\" as a quoted cmd-line
# argument (e.g. an unpatched %~dp0) can arrive here with a stray trailing
# quote from the C runtime's argument parsing -- strip it before resolving.
$LauncherRoot = (Resolve-Path -LiteralPath $LauncherRoot.TrimEnd('"')).Path.TrimEnd('\')

# ---------------------------------------------------------------------
# Repository config -- single source of truth is bootstrap/config/repo.conf.
# The embedded defaults below exist only so this one script still works if
# it was fetched standalone (e.g. via the raw-file fallback in
# OpenMindAI-Setup.bat) before the rest of the repo -- including that
# config file -- has been cloned yet. They must always match repo.conf.
# ---------------------------------------------------------------------
$RepoUrl = "https://github.com/smshagor-dev/OpenMindAI.git"
$RepoBranch = "main"
$RepoOwner = "smshagor-dev"
$RepoName = "OpenMindAI"

$RepoConfigPath = Join-Path $ScriptRoot "..\config\repo.conf"
if (Test-Path $RepoConfigPath) {
  Get-Content $RepoConfigPath | ForEach-Object {
    if ($_ -match '^\s*OPENMINDAI_REPO_URL=(.+)$') { $RepoUrl = $matches[1].Trim() }
    if ($_ -match '^\s*OPENMINDAI_REPO_BRANCH=(.+)$') { $RepoBranch = $matches[1].Trim() }
    if ($_ -match '^\s*OPENMINDAI_REPO_OWNER=(.+)$') { $RepoOwner = $matches[1].Trim() }
    if ($_ -match '^\s*OPENMINDAI_REPO_NAME=(.+)$') { $RepoName = $matches[1].Trim() }
  }
}

# ---------------------------------------------------------------------
# Internet detection -- quick, bounded, never blocks offline use.
# ---------------------------------------------------------------------
function Test-InternetAvailable {
  try {
    $client = New-Object System.Net.Sockets.TcpClient
    $connectTask = $client.ConnectAsync("github.com", 443)
    $completed = $connectTask.Wait(2500)
    $ok = $completed -and $client.Connected
    $client.Close()
    return [bool]$ok
  } catch {
    return $false
  }
}

# ---------------------------------------------------------------------
# Source resolution: if $LauncherRoot already looks like a real OpenMindAI
# checkout, use it directly. Otherwise the source lives in a subfolder
# (fresh clone target), keeping SOURCE_ROOT and the launcher's own location
# cleanly separate.
# ---------------------------------------------------------------------
function Test-ValidOpenMindAISource([string]$path) {
  if (-not (Test-Path $path)) { return $false }
  $required = @("package.json", "src", "src-tauri", "README.md")
  foreach ($item in $required) {
    if (-not (Test-Path (Join-Path $path $item))) { return $false }
  }
  return $true
}

function Resolve-SourceRoot {
  if (Test-ValidOpenMindAISource $LauncherRoot) {
    return $LauncherRoot
  }
  return (Join-Path $LauncherRoot "OpenMindAI")
}

# ---------------------------------------------------------------------
# Git detection / bootstrap.
# ---------------------------------------------------------------------
function Test-GitAvailable {
  $null = Get-Command git -ErrorAction SilentlyContinue
  return $?
}

function Install-Git {
  Write-Step "Git is required but was not found."
  if (Get-Command winget -ErrorAction SilentlyContinue) {
    Write-Info "Installing Git via winget (you may see a Windows permission prompt)..."
    try {
      winget install --id Git.Git -e --source winget --accept-source-agreements --accept-package-agreements
      # winget installs register PATH for new shells, not this running one.
      $gitPath = "$env:ProgramFiles\Git\cmd"
      if (Test-Path $gitPath) { $env:Path = "$gitPath;$env:Path" }
    } catch {
      Write-Warn "Automatic Git install via winget failed: $($_.Exception.Message)"
    }
  }
  if (-not (Test-GitAvailable)) {
    Write-Err "Git could not be installed automatically."
    Write-Info "Install it manually from https://git-scm.com/download/win, then run this setup again."
    throw "git not available"
  }
  Write-Info "Git installed."
}

# ---------------------------------------------------------------------
# Safe source sync: clone if missing; fetch + fast-forward only if the
# existing checkout is clean. Never `reset --hard` local changes -- if the
# checkout is dirty, warn and keep using it as-is (see spec: "safe git
# update").
# ---------------------------------------------------------------------
function Sync-Source([string]$sourceRoot) {
  if (-not (Test-Path (Join-Path $sourceRoot ".git"))) {
    if (Test-Path $sourceRoot) {
      if (Test-ValidOpenMindAISource $sourceRoot) {
        Write-Info "Existing OpenMindAI source found at $sourceRoot (not a git checkout) -- using as-is."
        return
      }
      throw "$sourceRoot exists but is not a valid OpenMindAI checkout and not empty. Remove it or choose a different location."
    }
    Write-Step "Cloning OpenMindAI source..."
    Write-Info "$RepoUrl -> $sourceRoot"
    & git clone --branch $RepoBranch $RepoUrl $sourceRoot
    if ($LASTEXITCODE -ne 0) { throw "git clone failed with exit code $LASTEXITCODE" }
    if (-not (Test-ValidOpenMindAISource $sourceRoot)) {
      throw "Cloned repository does not look like OpenMindAI (missing package.json/src/src-tauri/README.md) -- refusing to continue."
    }
    Write-Info "Source cloned."
    return
  }

  if (-not (Test-InternetAvailable)) {
    Write-Info "Offline -- skipping source update, using existing checkout."
    return
  }

  Push-Location $sourceRoot
  try {
    $status = & git status --porcelain 2>$null
    if ($LASTEXITCODE -ne 0) {
      Write-Warn "Could not check git status; skipping source update."
      return
    }
    if ($status) {
      Write-Warn "Existing source checkout has local changes -- skipping automatic update."
      Write-Info "(Not touching it automatically avoids discarding your changes. Update it yourself with 'git pull' if you want the latest source.)"
      return
    }
    Write-Step "Updating OpenMindAI source..."
    & git fetch origin $RepoBranch --quiet
    if ($LASTEXITCODE -ne 0) { Write-Warn "git fetch failed; using existing checkout."; return }
    & git merge --ff-only "origin/$RepoBranch" --quiet
    if ($LASTEXITCODE -ne 0) {
      Write-Warn "Fast-forward update not possible (local history has diverged); using existing checkout."
      return
    }
    Write-Info "Source up to date."
  } finally {
    Pop-Location
  }
}

# ---------------------------------------------------------------------
# Prebuilt release resolution (preferred path for normal users).
# ---------------------------------------------------------------------
function Resolve-PrebuiltRelease {
  if (-not (Test-InternetAvailable)) { return $null }
  try {
    $headers = @{ "User-Agent" = "OpenMindAI-Bootstrap" }
    $release = Invoke-RestMethod -UseBasicParsing -Headers $headers `
      -Uri "https://api.github.com/repos/$RepoOwner/$RepoName/releases/latest"
  } catch {
    Write-Info "No published GitHub release found yet (or it couldn't be reached) -- falling back to source build."
    return $null
  }
  $asset = $release.assets | Where-Object { $_.name -like "*x64*.exe" -or $_.name -like "*Setup*x64*" } | Select-Object -First 1
  if (-not $asset) {
    Write-Info "Latest release ($($release.tag_name)) has no Windows x64 installer asset -- falling back to source build."
    return $null
  }
  return [PSCustomObject]@{
    Version = $release.tag_name
    Name    = $asset.name
    Url     = $asset.browser_download_url
  }
}

function Install-PrebuiltRelease($asset, [string]$installDir) {
  Write-Step "Downloading OpenMindAI $($asset.Version) ($($asset.Name))..."
  New-Item -ItemType Directory -Force -Path $installDir | Out-Null
  $installerPath = Join-Path $installDir $asset.Name
  Invoke-WebRequest -UseBasicParsing -Uri $asset.Url -OutFile $installerPath

  $checksumAsset = $null
  try {
    $checksumAsset = (Invoke-RestMethod -UseBasicParsing -Uri "https://api.github.com/repos/$RepoOwner/$RepoName/releases/latest").assets |
      Where-Object { $_.name -eq "SHA256SUMS.txt" } | Select-Object -First 1
  } catch {}
  if ($checksumAsset) {
    Write-Info "Verifying checksum..."
    $sums = Invoke-WebRequest -UseBasicParsing -Uri $checksumAsset.browser_download_url | Select-Object -ExpandProperty Content
    $expected = ($sums -split "`n" | Where-Object { $_ -match [regex]::Escape($asset.Name) } | Select-Object -First 1) -split '\s+' | Select-Object -First 1
    $actual = (Get-FileHash -Path $installerPath -Algorithm SHA256).Hash
    if ($expected -and ($actual -notlike "$expected*")) {
      Remove-Item $installerPath -Force
      throw "Checksum mismatch for downloaded installer -- refusing to run it."
    }
    Write-Info "Checksum verified."
  } else {
    Write-Warn "No published checksum found for this release -- installing unverified."
  }

  Write-Step "Running installer..."
  Start-Process -FilePath $installerPath -Wait
  return $installerPath
}

# ---------------------------------------------------------------------
# Source build fallback (developer mode / no compatible release yet).
# ---------------------------------------------------------------------
function Get-CargoTargetDir([string]$sourceRoot) {
  Push-Location (Join-Path $sourceRoot "src-tauri")
  try {
    $metadata = & cargo metadata --no-deps --format-version 1 2>$null | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or -not $metadata) { return $null }
    return $metadata.target_directory
  } catch {
    return $null
  } finally {
    Pop-Location
  }
}

function Find-ExistingBuild([string]$sourceRoot) {
  $targetDir = Get-CargoTargetDir $sourceRoot
  if (-not $targetDir) { return $null }
  $exePath = Join-Path $targetDir "release\open-mind-ai.exe"
  if (Test-Path $exePath) { return $exePath }
  return $null
}

function Build-FromSource([string]$sourceRoot) {
  Write-Step "No prebuilt release available -- building OpenMindAI from source."
  Write-Info "This only happens once per machine (or when the source changes)."

  if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    throw "Node.js is required to build from source. Install it from https://nodejs.org/ and run setup again."
  }
  if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
    throw "npm is required to build from source (usually installed with Node.js)."
  }
  if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "Rust/Cargo is required to build from source. Install it from https://rustup.rs/ and run setup again."
  }

  $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
  if (-not (Test-Path $vswhere)) {
    throw "Microsoft Visual C++ Build Tools were not found (needed to compile the Rust backend). " +
      "Install them from https://visualstudio.microsoft.com/visual-cpp-build-tools/ (Desktop development with C++ workload), then run setup again."
  }
  $msvcEnv = Join-Path $sourceRoot "scripts\setup-msvc-env.ps1"
  if (Test-Path $msvcEnv) {
    . $msvcEnv
  }

  Push-Location $sourceRoot
  try {
    if (-not (Test-Path (Join-Path $sourceRoot "node_modules"))) {
      Write-Step "Installing frontend dependencies (first run only)..."
      # Piped to Out-Host (not left unredirected): an external command's
      # unredirected stdout is *also* pipeline output in PowerShell, and
      # since this function's result is captured by its caller
      # ($exePath = Build-FromSource ...), thousands of npm/cargo log lines
      # would otherwise get silently appended to the return value, turning
      # it into an array instead of the single path string it's meant to be.
      & npm install | Out-Host
      if ($LASTEXITCODE -ne 0) { throw "npm install failed" }
    }
    Write-Step "Building OpenMindAI (this can take several minutes)..."
    & npm run tauri -- build --no-bundle | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "build failed with exit code $LASTEXITCODE" }
  } finally {
    Pop-Location
  }

  $exePath = Find-ExistingBuild $sourceRoot
  if (-not $exePath) { throw "Build finished but the resulting executable could not be found." }
  return $exePath
}

# ---------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------
try {
  Write-Step "OpenMindAI Setup"
  Write-Info "Launcher: $LauncherRoot"

  $online = Test-InternetAvailable
  Write-Info $(if ($online) { "Internet: available" } else { "Internet: unavailable -- using what's already installed" })

  $sourceRoot = Resolve-SourceRoot

  if (-not (Test-ValidOpenMindAISource $sourceRoot)) {
    if (-not $online) {
      throw "OpenMindAI isn't installed yet and no internet connection is available to set it up. Connect to the internet and run setup again."
    }
    if (-not (Test-GitAvailable)) { Install-Git }
    Sync-Source $sourceRoot
  } else {
    Write-Info "Existing OpenMindAI source found at $sourceRoot"
    if ((Test-Path (Join-Path $sourceRoot ".git")) -and (Test-GitAvailable) -and $online) {
      Sync-Source $sourceRoot
    }
  }

  # Already built? Reuse it -- this is the fast "second run" / offline path.
  $existingExe = Find-ExistingBuild $sourceRoot
  $exePath = $null

  if ($existingExe -and -not $DeveloperMode) {
    Write-Info "Using existing OpenMindAI build."
    $exePath = $existingExe
  } elseif ($online) {
    $release = Resolve-PrebuiltRelease
    if ($release) {
      $installDir = Join-Path $LauncherRoot "release-download"
      Install-PrebuiltRelease $release $installDir | Out-Null
      Write-Step "OpenMindAI installed. Launch it from the Start Menu."
      if (-not $NoLaunch) { exit 0 }
    } else {
      $exePath = Build-FromSource $sourceRoot
    }
  } elseif ($existingExe) {
    $exePath = $existingExe
  } else {
    throw "No installed OpenMindAI build found, and no internet connection is available to install one."
  }

  if ($exePath -and -not $NoLaunch) {
    Write-Step "Starting OpenMindAI..."
    Start-Process -FilePath $exePath
  }

  Write-Host ""
  Write-Host "OpenMindAI is ready." -ForegroundColor Green
  exit 0
} catch {
  Write-Err $_.Exception.Message
  exit 1
}
