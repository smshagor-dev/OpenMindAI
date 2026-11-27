param(
  [string]$TestName,
  [string[]]$HarnessArgs = @(),
  [string]$TestRoot
)

$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent $PSScriptRoot
$tauriRoot = Join-Path $workspace 'src-tauri'
$depsDir = Join-Path $tauriRoot 'target\debug\deps'

if ([string]::IsNullOrWhiteSpace($TestRoot)) {
  throw 'Supply -TestRoot with a new directory under the OS temp directory named openmindai-rust-tests-*.'
}
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
$resolvedTestRoot = [IO.Path]::GetFullPath($TestRoot).TrimEnd('\')
if (-not $resolvedTestRoot.StartsWith($tempRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
  throw "TestRoot must be inside the OS temp directory ($tempRoot)."
}
if ((Split-Path -Leaf $resolvedTestRoot) -notlike 'openmindai-rust-tests-*') {
  throw 'TestRoot directory name must start with openmindai-rust-tests-.'
}
if (Test-Path -LiteralPath $resolvedTestRoot) {
  throw "TestRoot must not already exist; choose a fresh disposable path: $resolvedTestRoot"
}
New-Item -ItemType Directory -Path $resolvedTestRoot | Out-Null
$installConfig = Join-Path $resolvedTestRoot 'test-install.json'
$env:OPENMINDAI_ROOT = $resolvedTestRoot
$env:OPENMINDAI_EXPECTED_ROOT = $resolvedTestRoot
$env:OPENMINDAI_TEST_ROOT = $resolvedTestRoot
$env:OPENMINDAI_INSTALL_CONFIG = $installConfig

Write-Host "Rust test disposable root: $resolvedTestRoot"
Write-Host "Isolated install pointer: $installConfig"

Push-Location $tauriRoot
try {
  & cargo test --lib --no-run
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

  # Tauri's MockRuntime test dependency links the native menu stack. Cargo's
  # test executable lacks the Common Controls v6 manifest embedded in the app,
  # and Windows then rejects TaskDialogIndirect at process load time.
  $testExe = Get-ChildItem -LiteralPath $depsDir -Filter 'open_mind_ai_lib-*.exe' |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
  if (-not $testExe) { throw "Could not locate the compiled Rust library test executable in $depsDir" }

  $sdkRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
  $manifestTool = Get-ChildItem -LiteralPath $sdkRoot -Filter 'mt.exe' -Recurse |
    Where-Object { $_.FullName -match '\\x64\\mt\.exe$' } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
  if (-not $manifestTool) { throw 'Windows SDK mt.exe (x64) is required to prepare the test executable.' }

  $manifestPath = Join-Path $env:TEMP 'openmindai-rust-tests.manifest'
  @'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="OpenMindAI.Tests" version="1.0.0.0" processorArchitecture="*"/>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
'@ | Set-Content -LiteralPath $manifestPath -Encoding utf8

  & $manifestTool.FullName -manifest $manifestPath "-outputresource:$($testExe.FullName);#1"
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

  if ($TestName) {
    & $testExe.FullName $TestName @HarnessArgs
  } else {
    & $testExe.FullName @HarnessArgs
  }
  exit $LASTEXITCODE
}
finally {
  Pop-Location
}
