param(
  [Parameter(Mandatory = $true)]
  [string]$RuntimeDir,

  [ValidatePattern('^[0-9a-fA-F]{40}$')]
  [string]$ExpectedCommit,

  # Optional tiny GGUF suite; standalone smoke executable also supports chat models.
  [string]$ModelPath,

  # Internal child mode: each loader scenario needs a fresh process/module cache.
  [switch]$Probe,
  [switch]$DynamicBackends,
  [switch]$CpuOnly
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows -or -not [Environment]::Is64BitProcess) {
  throw 'Native runtime validation requires Windows x64 PowerShell 7'
}
$runtimeRoot = (Resolve-Path -LiteralPath $RuntimeDir).Path
if ($Probe) {
  Add-Type -Path (Join-Path $PSScriptRoot 'native-runtime-probe.cs')
  exit [NativeRuntimeProbe]::Run($runtimeRoot, $DynamicBackends, $CpuOnly)
}
if (-not $ExpectedCommit) {
  throw 'ExpectedCommit is required for package validation'
}
if ($ModelPath) {
  $ModelPath = (Resolve-Path -LiteralPath $ModelPath).Path
  $reportRoot = New-Item -ItemType Directory -Force 'native-smoke-reports'
}

$manifest = Get-Content -LiteralPath (Join-Path $runtimeRoot 'native-runtime-manifest.json') -Raw | ConvertFrom-Json
$loading = if ($manifest.PSObject.Properties['backendLoading']) { $manifest.backendLoading } else { 'linked' }
if ($loading -notin @('linked', 'dynamic')) { throw "Unsupported backend loading mode: $loading" }
$DynamicBackends = $loading -eq 'dynamic'
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

function Invoke-IsolatedProbe([string]$Directory, [int]$ExpectedExit, [string]$Scenario,
                              [string]$ExpectedOutput = '', [switch]$OnlyCpu,
                              [string]$WrapperMode = '', [string]$InferenceReport = '',
                              [switch]$ExpectGpuUnavailable) {
  $start = [Diagnostics.ProcessStartInfo]::new()
  $start.FileName = $shell
  $start.UseShellExecute = $false
  $start.RedirectStandardOutput = $true
  $start.RedirectStandardError = $true
  $start.WorkingDirectory = $scratch
  if ($InferenceReport) {
    $start.FileName = Join-Path $Directory 'native-inference-smoke.exe'
    foreach ($argument in @('--model', $ModelPath, '--timeout-seconds', '25',
                            '--report', (Join-Path $reportRoot.FullName $InferenceReport))) {
      $start.ArgumentList.Add($argument)
    }
    if ($ExpectGpuUnavailable) { $start.ArgumentList.Add('--expect-gpu-unavailable') }
  } elseif ($WrapperMode) {
    $start.FileName = Join-Path $Directory 'native-backend-probe.exe'
    $start.ArgumentList.Add($WrapperMode)
  } else {
    foreach ($argument in @('-NoLogo', '-NoProfile', '-NonInteractive', '-File', $PSCommandPath,
                          '-RuntimeDir', $Directory, '-Probe')) {
      $start.ArgumentList.Add($argument)
    }
    if ($DynamicBackends) { $start.ArgumentList.Add('-DynamicBackends') }
    if ($OnlyCpu) { $start.ArgumentList.Add('-CpuOnly') }
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
    $output = $stdout.GetAwaiter().GetResult()
    Write-Host $output
    Write-Host $stderr.GetAwaiter().GetResult()
    if ($process.ExitCode -ne $ExpectedExit) {
      throw "$Scenario failed: exit $($process.ExitCode), expected $ExpectedExit. Check packaged DLLs and system runtime prerequisites."
    }
    if ($ExpectedOutput -and -not $output.Contains($ExpectedOutput)) {
      throw "$Scenario did not exercise the expected fallback: $ExpectedOutput"
    }
    Write-Host "runtime.scenario: $Scenario passed"
  }
  finally { $process.Dispose() }
}

try {
  # The positive probe must pass before missing-DLL negatives can count as success.
  Invoke-IsolatedProbe $runtimeRoot 0 'packaged runtime without SDK search paths'
  if ($DynamicBackends) {
    Invoke-IsolatedProbe $runtimeRoot 0 'CPU-only initialization does not load Vulkan' -OnlyCpu
    Invoke-IsolatedProbe $runtimeRoot 0 'actual CXX CPU initialization' -WrapperMode cpu
  }
  if ($ModelPath) {
    Invoke-IsolatedProbe $runtimeRoot 0 'real GGUF CPU generation and cancellation' -InferenceReport 'cpu.json'
  }
  foreach ($missing in $required) {
    $damaged = Join-Path $scratch "missing-$missing"
    New-Item -ItemType Directory -Path $damaged | Out-Null
    foreach ($name in $names) {
      if ($name -ne $missing) {
        Copy-Item -LiteralPath (Join-Path $runtimeRoot $name) -Destination $damaged
      }
    }
    if ($DynamicBackends -and $missing -eq 'ggml-vulkan.dll') {
      Copy-Item -LiteralPath (Join-Path $runtimeRoot 'native-backend-probe.exe') -Destination $damaged
      Invoke-IsolatedProbe $damaged 0 'missing Vulkan plugin preserves CPU' 'runtime.vulkan: unavailable'
      Invoke-IsolatedProbe $damaged 0 'CXX reports missing Vulkan before model load' -WrapperMode gpu-unavailable
      Invoke-IsolatedProbe $damaged 0 'CXX CPU works without Vulkan plugin' -WrapperMode cpu
      if ($ModelPath) {
        Copy-Item -LiteralPath (Join-Path $runtimeRoot 'native-inference-smoke.exe') -Destination $damaged
        Invoke-IsolatedProbe $damaged 0 'real generation after missing Vulkan plugin' `
          -InferenceReport 'missing-vulkan.json' -ExpectGpuUnavailable
      }
    } else {
      Invoke-IsolatedProbe $damaged 20 "missing $missing is detected by loader"
    }
  }
  if ($DynamicBackends -and $manifest.backend -eq 'vulkan') {
    $damaged = Join-Path $scratch 'missing-vulkan-loader'
    New-Item -ItemType Directory -Path $damaged | Out-Null
    foreach ($name in $names) {
      Copy-Item -LiteralPath (Join-Path $runtimeRoot $name) -Destination $damaged
    }
    if (Test-Path -LiteralPath (Join-Path $env:SystemRoot 'System32/omai-mis.dll')) {
      throw 'Fault injection DLL name unexpectedly exists on this host'
    }
    Add-Type -Path (Join-Path $PSScriptRoot 'native-runtime-probe.cs')
    [NativeRuntimeProbe]::BreakVulkanImport((Join-Path $damaged 'ggml-vulkan.dll'))
    Copy-Item -LiteralPath (Join-Path $runtimeRoot 'native-backend-probe.exe') -Destination $damaged
    Invoke-IsolatedProbe $damaged 0 'missing Vulkan loader preserves CPU' 'runtime.vulkan: unavailable'
    Invoke-IsolatedProbe $damaged 0 'CXX reports missing Vulkan loader before model load' -WrapperMode gpu-unavailable
    Invoke-IsolatedProbe $damaged 0 'CXX CPU works without Vulkan loader' -WrapperMode cpu
    if ($ModelPath) {
      Copy-Item -LiteralPath (Join-Path $runtimeRoot 'native-inference-smoke.exe') -Destination $damaged
      Invoke-IsolatedProbe $damaged 0 'real generation after missing Vulkan loader' `
        -InferenceReport 'missing-loader.json' -ExpectGpuUnavailable
    }
  }
}
finally { Remove-Item -LiteralPath $scratch -Recurse -Force }
