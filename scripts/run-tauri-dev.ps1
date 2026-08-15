param(
  # Defaults to a sibling of the repo, never inside it — pointing this at the
  # repo root itself used to make dev runs write multi-GB runtime/model data
  # straight into the source tree. Pass -PortableRoot to use a different root.
  [string]$PortableRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\..\OpenMindAI-data"))
)

$ErrorActionPreference = "Stop"
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$env:OPENMINDAI_ROOT = $PortableRoot

. (Join-Path $PSScriptRoot "setup-msvc-env.ps1")

npm run tauri dev
