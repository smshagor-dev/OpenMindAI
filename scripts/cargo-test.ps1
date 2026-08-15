param(
  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]]$CargoArgs
)

$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "setup-msvc-env.ps1")

Push-Location (Join-Path $PSScriptRoot "..\src-tauri")
try {
  cargo test @CargoArgs
} finally {
  Pop-Location
}
