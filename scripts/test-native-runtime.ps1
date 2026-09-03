param(
  [Parameter(Mandatory = $true)]
  [string]$RuntimeDir,

  [ValidatePattern('^[0-9a-fA-F]{40}$')]
  [string]$ExpectedCommit,

  # Internal child mode: each loader scenario needs a fresh process/module cache.
  [switch]$Probe
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows -or -not [Environment]::Is64BitProcess) {
  throw 'Native runtime validation requires Windows x64 PowerShell 7'
}
$runtimeRoot = (Resolve-Path -LiteralPath $RuntimeDir).Path
if ($Probe) {
  Add-Type -Path (Join-Path $PSScriptRoot 'native-runtime-probe.cs')
  exit [NativeRuntimeProbe]::Run($runtimeRoot)
}
if (-not $ExpectedCommit) {
  throw 'ExpectedCommit is required for package validation'
}

$manifest = Get-Content -LiteralPath (Join-Path $runtimeRoot 'native-runtime-manifest.json') -Raw | ConvertFrom-Json
$commit = $ExpectedCommit.ToLowerInvariant()
if ($manifest.schemaVersion -ne 1 -or $manifest.llamaCppCommit -ne $commit -or
    $manifest.abiTag -ne "llama-cxx-$($commit.Substring(0, 12))" -or
    $manifest.platform -ne 'windows' -or $manifest.architecture -ne 'x86_64' -or
    $manifest.linkMode -ne 'shared' -or $manifest.backend -notin @('cpu', 'vulkan')) {
  throw 'Native runtime manifest does not match the expected Windows ABI contract'
}
$names = @()
foreach ($file in $manifest.files) {
  if ($file.name -notmatch '^[A-Za-z0-9_.-]+\.dll$' -or $file.name -in $names) {
    throw "Invalid or duplicate manifest DLL name: $($file.name)"
  }
  $names += $file.name
  $path = Join-Path $runtimeRoot $file.name
  if ((Get-Item -LiteralPath $path).Length -ne $file.bytes -or
      (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $file.sha256) {
    throw "Native runtime integrity check failed: $($file.name)"
  }
}
$required = @('llama.dll', 'ggml.dll', 'ggml-base.dll', 'ggml-cpu.dll')
if ($manifest.backend -eq 'vulkan') { $required += 'ggml-vulkan.dll' }
foreach ($name in $required) {
  if ($name -notin $names) { throw "Required DLL is missing from manifest: $name" }
}
$actual = @(Get-ChildItem -LiteralPath $runtimeRoot -File -Filter '*.dll' | ForEach-Object Name)
if (@(Compare-Object $names $actual).Count -ne 0) {
  throw 'Bundle contains DLLs outside the validated manifest'
}
Write-Host "runtime.manifest: passed ($($manifest.backend), $($manifest.abiTag))"

$scratch = Join-Path ([IO.Path]::GetTempPath()) ("openmind-runtime-probe-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $scratch | Out-Null
$shell = (Get-Process -Id $PID).Path

function Invoke-IsolatedProbe([string]$Directory, [int]$ExpectedExit, [string]$Scenario) {
  $start = [Diagnostics.ProcessStartInfo]::new()
  $start.FileName = $shell
  $start.UseShellExecute = $false
  $start.RedirectStandardOutput = $true
  $start.RedirectStandardError = $true
  $start.WorkingDirectory = $scratch
  foreach ($argument in @('-NoLogo', '-NoProfile', '-NonInteractive', '-File', $PSCommandPath,
                          '-RuntimeDir', $Directory, '-Probe')) {
    $start.ArgumentList.Add($argument)
  }
  # Keep OS/driver support, but remove all SDK/build paths and Vulkan overrides.
  $start.Environment['PATH'] = "$env:SystemRoot\System32;$env:SystemRoot"
  foreach ($key in @($start.Environment.Keys)) {
    if ($key -match '^(VULKAN|VK_|GGML_|LLAMA_CPP|OPENMINDAI_NATIVE)') {
      [void]$start.Environment.Remove($key)
    }
  }
  $process = [Diagnostics.Process]::new()
  $process.StartInfo = $start
  try {
    [void]$process.Start()
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(30000)) {
      $process.Kill($true)
      $process.WaitForExit()
      throw "Runtime probe timed out: $Scenario"
    }
    Write-Host $stdout.GetAwaiter().GetResult()
    Write-Host $stderr.GetAwaiter().GetResult()
    if ($process.ExitCode -ne $ExpectedExit) {
      throw "$Scenario failed: exit $($process.ExitCode), expected $ExpectedExit. Check packaged DLLs and system runtime prerequisites."
    }
    Write-Host "runtime.scenario: $Scenario passed"
  }
  finally { $process.Dispose() }
}

try {
  # The positive probe must pass before missing-DLL negatives can count as success.
  Invoke-IsolatedProbe $runtimeRoot 0 'packaged runtime without SDK search paths'
  foreach ($missing in $required) {
    $damaged = Join-Path $scratch "missing-$missing"
    New-Item -ItemType Directory -Path $damaged | Out-Null
    foreach ($name in $names) {
      if ($name -ne $missing) {
        Copy-Item -LiteralPath (Join-Path $runtimeRoot $name) -Destination $damaged
      }
    }
    Invoke-IsolatedProbe $damaged 20 "missing $missing is detected by loader"
  }
}
finally { Remove-Item -LiteralPath $scratch -Recurse -Force }
