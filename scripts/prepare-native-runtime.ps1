param(
  [Parameter(Mandatory = $true)]
  [string]$LlamaBuildDir,

  [Parameter(Mandatory = $true)]
  [string]$OutputDir,

  [Parameter(Mandatory = $true)]
  [ValidatePattern('^[0-9a-fA-F]{40}$')]
  [string]$Commit,

  [string]$Platform = 'windows',
  [string]$Architecture = 'x86_64'
)

$ErrorActionPreference = 'Stop'

$buildRoot = (Resolve-Path -LiteralPath $LlamaBuildDir).Path
$llamaDll = Get-ChildItem -LiteralPath $buildRoot -Filter 'llama.dll' -File -Recurse |
  Where-Object { $_.FullName -match '(?i)(\\|/)bin(\\|/)' } |
  Sort-Object FullName |
  Select-Object -First 1

if ($null -eq $llamaDll) {
  throw "Unable to locate llama.dll under $buildRoot"
}

$libraryDir = $llamaDll.Directory.FullName
$required = @('llama.dll', 'ggml.dll', 'ggml-base.dll', 'ggml-cpu.dll')
foreach ($name in $required) {
  $path = Join-Path $libraryDir $name
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
    throw "Native runtime is incomplete: missing $name beside $($llamaDll.FullName)"
  }
}

if (Test-Path -LiteralPath $OutputDir) {
  Remove-Item -LiteralPath $OutputDir -Recurse -Force
}
New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null

$dlls = @(Get-ChildItem -LiteralPath $libraryDir -Filter '*.dll' -File | Sort-Object Name)
if ($dlls.Count -lt $required.Count) {
  throw "Native runtime DLL set is unexpectedly small: found $($dlls.Count) files"
}

$manifestFiles = @()
foreach ($dll in $dlls) {
  $destination = Join-Path $OutputDir $dll.Name
  Copy-Item -LiteralPath $dll.FullName -Destination $destination -Force
  $hash = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
  $manifestFiles += [ordered]@{
    name = $dll.Name
    sha256 = $hash
    bytes = $dll.Length
  }
}

$normalizedCommit = $Commit.ToLowerInvariant()
$abiTag = "llama-cxx-$($normalizedCommit.Substring(0, 12))"
$manifest = [ordered]@{
  schemaVersion = 1
  abiTag = $abiTag
  llamaCppCommit = $normalizedCommit
  platform = $Platform
  architecture = $Architecture
  linkMode = 'shared'
  files = $manifestFiles
}

$manifestPath = Join-Path $OutputDir 'native-runtime-manifest.json'
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestPath -Encoding utf8

Write-Host "Prepared native runtime: $abiTag"
Write-Host "Library directory: $libraryDir"
Write-Host "Bundle directory: $OutputDir"
Write-Host "Manifest: $manifestPath"
Write-Output $libraryDir
