param(
  [Parameter(Mandatory = $true)][string]$IdentityFile,
  [Parameter(Mandatory = $true)][string]$OutputFile
)

$ErrorActionPreference = 'Stop'
$identity = Get-Content -LiteralPath $IdentityFile -Raw | ConvertFrom-Json
$app = Get-CimInstance Win32_Process -Filter "ProcessId=$($identity.pid)"
if (-not $app -or $app.ExecutablePath -ne $identity.executable) {
  throw 'The recorded application PID is not running the recorded executable.'
}
if ((Get-FileHash -LiteralPath $identity.executable -Algorithm SHA256).Hash -ne $identity.sha256) {
  throw 'The executable SHA-256 no longer matches the launch record.'
}
$webview = Get-CimInstance Win32_Process -Filter "ProcessId=$($identity.webviewPid)"
if (-not $webview -or [int]$webview.ParentProcessId -ne [int]$identity.pid) {
  throw 'The recorded WebView is not a child of the recorded application process.'
}
if ($webview.CommandLine -notmatch [regex]::Escape($identity.webviewProfile)) {
  throw 'The WebView profile does not match the unique profile in the launch record.'
}
$listener = Get-NetTCPConnection -LocalPort ([int]$identity.debugPort) -State Listen -ErrorAction SilentlyContinue |
  Where-Object { [int]$_.OwningProcess -eq [int]$identity.webviewPid } |
  Select-Object -First 1
if (-not $listener) {
  throw 'The recorded debug port is not owned by the recorded WebView process.'
}

$nodeScript = Join-Path (Split-Path -Parent $PSScriptRoot) 'scripts/probe-native-tauri-bridge.mjs'
$output = & node $nodeScript ([int]$identity.debugPort) $OutputFile
if ($LASTEXITCODE -ne 0) {
  throw "Native WebView bridge probe failed. Evidence: $OutputFile"
}
$appAfterProbe = Get-CimInstance Win32_Process -Filter "ProcessId=$($identity.pid)"
if (-not $appAfterProbe -or $appAfterProbe.ExecutablePath -ne $identity.executable) {
  throw 'The recorded application process exited or changed identity during the probe.'
}
$output
