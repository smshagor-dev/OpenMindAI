# Imports the MSVC (Visual C++ Build Tools) environment into the current
# PowerShell session, so `cargo build` / `cargo test` can find link.exe.
#
# Needed because Git for Windows ships its own usr/bin/link.exe (a Unix
# hardlink tool), which shadows the real MSVC linker on PATH otherwise.
# Dot-source this before running cargo directly:
#   . .\scripts\setup-msvc-env.ps1
$ErrorActionPreference = "Stop"

$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path $vswhere)) {
  throw "vswhere.exe not found - is Visual Studio / Build Tools installed?"
}

$installPath = & $vswhere -latest -products * `
  -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
  -property installationPath
if (-not $installPath) {
  throw "No Visual Studio install with the C++ Build Tools workload was found. Run: winget install --id Microsoft.VisualStudio.2022.BuildTools --override `"--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended`""
}

$vcvars = Join-Path $installPath "VC\Auxiliary\Build\vcvars64.bat"
if (-not (Test-Path $vcvars)) {
  throw "vcvars64.bat not found at expected path: $vcvars"
}

# vcvars64.bat internally shells out to a *bare* `vswhere.exe` (relying on
# PATH) to re-detect the install -- put its directory on PATH for this
# process, or that inner call fails even though we already resolved it above.
$env:PATH = "$(Split-Path $vswhere);$env:PATH"

# Run vcvars64.bat in cmd.exe, dump the resulting environment, and import
# each variable into this PowerShell process.
$envDump = cmd /c "`"$vcvars`" && set"
foreach ($line in $envDump) {
  if ($line -match '^([^=]+)=(.*)$') {
    [System.Environment]::SetEnvironmentVariable($matches[1], $matches[2], "Process")
  }
}

Write-Host "MSVC environment loaded from: $installPath" -ForegroundColor Green
