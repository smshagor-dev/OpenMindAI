$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "setup-msvc-env.ps1")

$repoRoot = Join-Path $PSScriptRoot ".."
Push-Location $repoRoot
try {
  $localUnsignedConfig = Join-Path $repoRoot "src-tauri\tauri.local-unsigned.conf.json"
  if ([string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY)) {
    $config = @{
      bundle = @{
        createUpdaterArtifacts = $false
      }
    } | ConvertTo-Json -Depth 10
    Set-Content -Path $localUnsignedConfig -Value $config -Encoding utf8
    npm run tauri -- build --config $localUnsignedConfig
  } else {
    npm run build:release
  }
  if ($LASTEXITCODE -ne 0) {
    throw "Tauri installer build failed with exit code $LASTEXITCODE"
  }
} finally {
  if (Test-Path $localUnsignedConfig) {
    Remove-Item -LiteralPath $localUnsignedConfig -Force
  }
  Pop-Location
}

# Don't assume `src-tauri/target` -- a machine-local `.cargo/config.toml`
# (or the CARGO_TARGET_DIR env var) can redirect Cargo's output elsewhere.
# Cargo's config discovery walks up from the current directory, not from
# --manifest-path, so this must run from inside src-tauri to pick up its
# .cargo/config.toml.
Push-Location (Join-Path $repoRoot "src-tauri")
try {
  $metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
} finally {
  Pop-Location
}
$bundleDir = Join-Path $metadata.target_directory "release\bundle\nsis"
if (-not (Test-Path $bundleDir)) {
  throw "Expected NSIS bundle output at $bundleDir but it does not exist"
}

$packageJson = Get-Content -Path (Join-Path $repoRoot "package.json") -Raw | ConvertFrom-Json
$version = $packageJson.version
$artifacts = Get-ChildItem -Path $bundleDir -File | Where-Object {
  ($_.Extension -in ".exe", ".sig") -and $_.Name -like "*_$version`_*"
}
if ($artifacts.Count -eq 0) {
  throw "No v$version installer artifacts found under $bundleDir"
}

$checksumPath = Join-Path $bundleDir "SHA256SUMS.txt"
$lines = foreach ($artifact in $artifacts) {
  $hash = (Get-FileHash -Path $artifact.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  "$hash  $($artifact.Name)"
}
# ASCII, not UTF-8 -- Windows PowerShell 5.1's -Encoding utf8 writes a BOM,
# which breaks standard `sha256sum -c` tooling.
Set-Content -Path $checksumPath -Value $lines -Encoding ascii

# Keep compiled distribution artifacts out of the Git source root. The
# release-output directory is gitignored and mirrors what CI uploads to a
# GitHub Release.
$releaseOutput = Join-Path $repoRoot "release-output"
if (Test-Path $releaseOutput) {
  Remove-Item -LiteralPath $releaseOutput -Recurse -Force
}
New-Item -ItemType Directory -Path $releaseOutput -Force | Out-Null

foreach ($artifact in $artifacts) {
  Copy-Item -LiteralPath $artifact.FullName -Destination $releaseOutput -Force
}
Copy-Item -LiteralPath $checksumPath -Destination $releaseOutput -Force

Write-Host ""
Write-Host "Installer build complete:" -ForegroundColor Green
foreach ($artifact in $artifacts) {
  Write-Host "  $($artifact.FullName)"
}
Write-Host "  $checksumPath"
Write-Host ""
Write-Host "Release files staged in:" -ForegroundColor Green
Write-Host "  $releaseOutput"
Write-Host ""
Write-Host "For public distribution, publish these files through the signed GitHub Release workflow." -ForegroundColor Yellow
