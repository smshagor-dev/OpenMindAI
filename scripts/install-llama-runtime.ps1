param(
  [string]$PortableRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path,
  [ValidateSet("cpu", "vulkan", "cuda12", "cuda13", "sycl", "hip")]
  [string]$Backend = "vulkan"
)

$ErrorActionPreference = "Stop"

$backendPatterns = @{
  cpu = "llama-*-bin-win-cpu-x64.zip"
  vulkan = "llama-*-bin-win-vulkan-x64.zip"
  cuda12 = "llama-*-bin-win-cuda-12.4-x64.zip"
  cuda13 = "llama-*-bin-win-cuda-13.3-x64.zip"
  sycl = "llama-*-bin-win-sycl-x64.zip"
  hip = "llama-*-bin-win-rocm-*-x64.zip"
}

function Get-RelativePath([string]$BasePath, [string]$FullPath) {
  $baseUri = [Uri]((Resolve-Path $BasePath).Path.TrimEnd('\') + '\')
  $fullUri = [Uri]((Resolve-Path $FullPath).Path)
  [Uri]::UnescapeDataString($baseUri.MakeRelativeUri($fullUri).ToString()).Replace('/', '\')
}

$release = Invoke-RestMethod `
  -Uri "https://api.github.com/repos/ggml-org/llama.cpp/releases/latest" `
  -Headers @{ "User-Agent" = "OpenMindAi" }

$pattern = $backendPatterns[$Backend]
$asset = $release.assets | Where-Object { $_.name -like $pattern } | Select-Object -First 1
if (-not $asset) {
  throw "No official llama.cpp release asset matched $pattern in $($release.tag_name)."
}

$runtimeRoot = Join-Path $PortableRoot "runtimes\llama"
$versionDir = Join-Path $runtimeRoot "$Backend\$($release.tag_name)"
$manifestDir = Join-Path $runtimeRoot "manifests"
$tempDir = Join-Path $PortableRoot "temp\downloads"
New-Item -ItemType Directory -Force -Path $versionDir,$manifestDir,$tempDir | Out-Null

$drive = Get-PSDrive -Name ([IO.Path]::GetPathRoot($PortableRoot).Substring(0, 1))
$safeMarginBytes = 2GB
if (($drive.Free - $asset.size) -lt $safeMarginBytes) {
  throw "Insufficient storage. Need package plus 2GB safety margin."
}

$zipPath = Join-Path $tempDir $asset.name
$partPath = "$zipPath.part"
Remove-Item $partPath -ErrorAction SilentlyContinue

Write-Host "Downloading official llama.cpp $Backend runtime $($release.tag_name)..."
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $partPath
Move-Item -Force $partPath $zipPath

$extractTemp = Join-Path $tempDir "$($asset.name).extract"
Remove-Item -Recurse -Force $extractTemp -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $extractTemp | Out-Null
Expand-Archive -LiteralPath $zipPath -DestinationPath $extractTemp -Force

$server = Get-ChildItem -Path $extractTemp -Recurse -Filter "llama-server.exe" | Select-Object -First 1
$cli = Get-ChildItem -Path $extractTemp -Recurse -Filter "llama-cli.exe" | Select-Object -First 1
$bench = Get-ChildItem -Path $extractTemp -Recurse -Filter "llama-bench.exe" | Select-Object -First 1
if (-not $server -and -not $cli) {
  throw "Archive did not contain llama-server.exe or llama-cli.exe."
}

Get-ChildItem -Path $extractTemp -Force | Move-Item -Destination $versionDir -Force
Remove-Item -Recurse -Force $extractTemp

$serverInstalled = Get-ChildItem -Path $versionDir -Recurse -Filter "llama-server.exe" | Select-Object -First 1
$cliInstalled = Get-ChildItem -Path $versionDir -Recurse -Filter "llama-cli.exe" | Select-Object -First 1
$benchInstalled = Get-ChildItem -Path $versionDir -Recurse -Filter "llama-bench.exe" | Select-Object -First 1

$manifest = [ordered]@{
  runtimeName = "llama.cpp $Backend"
  version = $release.tag_name
  platform = "windows"
  architecture = "x86_64"
  backend = $Backend
  source = $asset.browser_download_url
  installedAt = (Get-Date).ToUniversalTime().ToString("o")
  binaries = [ordered]@{
    server = if ($serverInstalled) { Get-RelativePath $PortableRoot $serverInstalled.FullName } else { $null }
    cli = if ($cliInstalled) { Get-RelativePath $PortableRoot $cliInstalled.FullName } else { $null }
    bench = if ($benchInstalled) { Get-RelativePath $PortableRoot $benchInstalled.FullName } else { $null }
  }
  checksum = $asset.digest
  status = "available"
}

$manifestPath = Join-Path $manifestDir "llama-$Backend-$($release.tag_name).json"
$manifest | ConvertTo-Json -Depth 8 | Set-Content -Encoding UTF8 -Path $manifestPath
Remove-Item $zipPath -Force

Write-Host "Installed runtime under $versionDir"
Write-Host "Manifest: $manifestPath"
