param(
  [string]$InstallerPath = (Join-Path $PSScriptRoot "..\release-output\OpenMindAI_3.0.0_x64-setup.exe"),
  [string]$InstallRoot = (Join-Path $env:TEMP "OpenMindAI-v3-local-smoke")
)

$ErrorActionPreference = "Stop"

$installer = (Resolve-Path -LiteralPath $InstallerPath).Path
$tempBase = [System.IO.Path]::GetFullPath($env:TEMP)
$targetRoot = [System.IO.Path]::GetFullPath($InstallRoot)

if (-not $targetRoot.StartsWith($tempBase, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw "Smoke install path must stay under temp. Resolved path: $targetRoot"
}

if (Test-Path -LiteralPath $targetRoot) {
  Remove-Item -LiteralPath $targetRoot -Recurse -Force
}
New-Item -ItemType Directory -Path $targetRoot -Force | Out-Null

try {
  $install = Start-Process -FilePath $installer -ArgumentList @("/S", "/D=$targetRoot") -Wait -PassThru -WindowStyle Hidden
  if ($install.ExitCode -ne 0) {
    throw "Installer failed with exit code $($install.ExitCode)"
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
  if (($productVersion -notlike "3.0.0*") -and ($fileVersion -notlike "3.0.0*")) {
    throw "Installed executable version mismatch. Expected 3.0.0; product=$productVersion file=$fileVersion path=$($appExe.FullName)"
  }

  Write-Host "Smoke install OK: $($appExe.FullName)"
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
