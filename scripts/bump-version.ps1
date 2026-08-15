param(
  [Parameter(Mandatory = $true)]
  [string]$Version
)

$ErrorActionPreference = "Stop"

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
  throw "Version must be MAJOR.MINOR.PATCH (e.g. 1.0.0), got '$Version'"
}

$repoRoot = Join-Path $PSScriptRoot ".."
$targets = @(
  @{ Path = Join-Path $repoRoot "package.json"; Pattern = '(?m)^(\s*"version":\s*")[^"]+(")' },
  @{ Path = Join-Path $repoRoot "src-tauri\tauri.conf.json"; Pattern = '(?m)^(\s*"version":\s*")[^"]+(")' },
  @{ Path = Join-Path $repoRoot "src-tauri\Cargo.toml"; Pattern = '(?m)^(version = ")[^"]+(")' }
)

# Read + validate every file has exactly one match before writing anything,
# so a bad target never leaves the three files bumped inconsistently.
$originals = @{}
foreach ($target in $targets) {
  $content = Get-Content -Path $target.Path -Raw
  $matches = [regex]::Matches($content, $target.Pattern)
  if ($matches.Count -ne 1) {
    throw "Expected exactly one version field in $($target.Path), found $($matches.Count)"
  }
  $originals[$target.Path] = $content
}

foreach ($target in $targets) {
  $updated = [regex]::Replace($originals[$target.Path], $target.Pattern, "`${1}$Version`${2}")
  Set-Content -Path $target.Path -Value $updated -NoNewline
  Write-Host "Updated $($target.Path) -> $Version"
}
