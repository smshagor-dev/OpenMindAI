param(
  [Parameter(Mandatory=$true)][string]$Exe,
  [Parameter(Mandatory=$true)][string]$Root,
  [Parameter(Mandatory=$true)][int]$Port,
  [Parameter(Mandatory=$true)][string]$IdentityOut
)
$ErrorActionPreference = 'Stop'

$profile = Join-Path $Root 'webview-profile'
$env:OPENMINDAI_ROOT = $Root
$env:WEBVIEW2_USER_DATA_FOLDER = $profile
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Port"

$proc = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Seconds 2

# Wait up to 60s for the CDP endpoint to answer.
$ready = $false
for ($i = 0; $i -lt 60; $i++) {
  try {
    $r = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/json/version" -UseBasicParsing -TimeoutSec 2
    if ($r.StatusCode -eq 200) { $ready = $true; break }
  } catch { Start-Sleep -Seconds 1 }
}
if (-not $ready) { throw "CDP endpoint on port $Port did not become ready" }

# Identify the WebView2 child renderer process (holds the debug port).
$webviewPid = $null
try {
  $conn = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($conn) { $webviewPid = $conn.OwningProcess }
} catch {}

$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $Exe).Hash
$identity = [ordered]@{
  capturedAtUtc = (Get-Date).ToUniversalTime().ToString('o')
  executable = (Resolve-Path $Exe).Path
  sha256 = $hash
  pid = $proc.Id
  webviewPid = $webviewPid
  root = (Resolve-Path $Root).Path
  database = (Join-Path (Resolve-Path $Root).Path 'data\database\openmind_ai.db')
  webviewProfile = $profile
  debugPort = $Port
}
$identity | ConvertTo-Json | Set-Content -LiteralPath $IdentityOut -Encoding utf8
$identity | ConvertTo-Json
