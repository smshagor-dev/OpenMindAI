$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "setup-msvc-env.ps1")

$repoRoot = Join-Path $PSScriptRoot ".."
Push-Location $repoRoot
try {
  npm run build:release
  if ($LASTEXITCODE -ne 0) {
    throw "npm run build:release failed with exit code $LASTEXITCODE"
  }
} finally {
  Pop-Location
}

# Don't assume `src-tauri/target` -- a machine-local `.cargo/config.toml`
# (or the CARGO_TARGET_DIR env var) can redirect Cargo's output elsewhere,
# as it does on this dev machine (G: is exFAT, unreliable for build output).
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
# which breaks standard `sha256sum -c` / CI tooling that expects a plain
# checksum file. Hex hashes and these filenames are pure ASCII anyway.
Set-Content -Path $checksumPath -Value $lines -Encoding ascii

$rootArtifacts = @()
foreach ($artifact in $artifacts) {
  $destination = Join-Path $repoRoot $artifact.Name
  Copy-Item -LiteralPath $artifact.FullName -Destination $destination -Force
  $rootArtifacts += $destination
}
$rootChecksumPath = Join-Path $repoRoot "SHA256SUMS.txt"
Copy-Item -LiteralPath $checksumPath -Destination $rootChecksumPath -Force

Write-Host ""
Write-Host "Installer build complete:" -ForegroundColor Green
foreach ($artifact in $artifacts) {
  Write-Host "  $($artifact.FullName)"
}
Write-Host "  $checksumPath"
Write-Host ""
Write-Host "Copied release files to project root:" -ForegroundColor Green
foreach ($artifact in $rootArtifacts) {
  Write-Host "  $artifact"
}
Write-Host "  $rootChecksumPath"
Write-Host ""
Write-Host "This build is unsigned (no Windows code-signing certificate configured yet)." -ForegroundColor Yellow
Write-Host "Windows SmartScreen will warn on first run -- verify the checksum above instead."
