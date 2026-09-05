param(
  [string]$InstallerPath = (Join-Path $PSScriptRoot "..\release-output\OpenMindAI_3.0.1_x64-setup.exe"),
  [string]$InstallRoot = (Join-Path $env:TEMP "OpenMindAI-v3.0.1-local-smoke"),
  [string]$ExpectedVersion,
  [switch]$ExerciseUpgrade
)

$ErrorActionPreference = "Stop"

$installer = (Resolve-Path -LiteralPath $InstallerPath).Path
$tempBase = [System.IO.Path]::GetFullPath($env:TEMP)
$targetRoot = [System.IO.Path]::GetFullPath($InstallRoot)
if (-not $ExpectedVersion) {
  $installerName = [System.IO.Path]::GetFileName($installer)
  if ($installerName -match '_(\d+\.\d+\.\d+)(?:[_-]|$)') {
    $ExpectedVersion = $Matches[1]
  } else {
    throw "ExpectedVersion is required when it cannot be inferred from installer name: $installerName"
  }
}

if (-not $targetRoot.StartsWith($tempBase, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw "Smoke install path must stay under temp. Resolved path: $targetRoot"
}

if (Test-Path -LiteralPath $targetRoot) {
  Remove-Item -LiteralPath $targetRoot -Recurse -Force
}
New-Item -ItemType Directory -Path $targetRoot -Force | Out-Null

try {
  function Invoke-SmokeInstall([string]$Label) {
    $install = Start-Process -FilePath $installer -ArgumentList @("/S", "/D=$targetRoot") -Wait -PassThru -WindowStyle Hidden
    if ($install.ExitCode -ne 0) {
      throw "$Label installer failed with exit code $($install.ExitCode)"
    }
    Write-Host "$Label install OK"
  }

  Invoke-SmokeInstall "Clean"
  if ($ExerciseUpgrade) {
    Invoke-SmokeInstall "Upgrade"
  }

  $appExe = Get-ChildItem -LiteralPath $targetRoot -Filter "*.exe" -File -Recurse |
    Where-Object { $_.Name -notmatch "(?i)uninstall" } |
    Sort-Object Length -Descending |
    Select-Object -First 1
  if ($null -eq $appExe) {
    throw "Installed app executable was not found under $targetRoot"
  }

  $productVersion = $appExe.VersionInfo.ProductVersion
  $fileVersion = $appExe.VersionInfo.FileVersion
  if (($productVersion -notlike "$ExpectedVersion*") -and ($fileVersion -notlike "$ExpectedVersion*")) {
    throw "Installed executable version mismatch. Expected $ExpectedVersion; product=$productVersion file=$fileVersion path=$($appExe.FullName)"
  }

  Write-Host "Smoke install OK: $($appExe.FullName)"
  Write-Host "ExpectedVersion: $ExpectedVersion"
  Write-Host "ProductVersion: $productVersion"
  Write-Host "FileVersion: $fileVersion"
} finally {
  $uninstaller = Get-ChildItem -LiteralPath $targetRoot -Filter "*.exe" -File -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match "(?i)uninstall" } |
    Select-Object -First 1
  if ($null -ne $uninstaller) {
    $uninstall = Start-Process -FilePath $uninstaller.FullName -ArgumentList "/S" -Wait -PassThru -WindowStyle Hidden
    if ($uninstall.ExitCode -ne 0) {
      throw "Uninstaller failed with exit code $($uninstall.ExitCode): $($uninstaller.FullName)"
    }
  }
  if (Test-Path -LiteralPath $targetRoot) {
    Remove-Item -LiteralPath $targetRoot -Recurse -Force
  }
}
