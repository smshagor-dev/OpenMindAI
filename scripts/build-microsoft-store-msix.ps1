param(
  [string]$PackageName = "OpenMindAI",
  [string]$Publisher = "CN=Open Mind AI",
  [string]$PublisherDisplayName = "Open Mind AI",
  [string]$OutputDir = "release-output\microsoftstore-msix",
  [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

function Get-MakeAppx {
  $roots = @(
    "C:\Program Files (x86)\Windows Kits\10\bin",
    "C:\Program Files\Windows Kits\10\bin"
  ) | Where-Object { Test-Path -LiteralPath $_ }

  $tools = foreach ($root in $roots) {
    Get-ChildItem -LiteralPath $root -Recurse -File -Filter MakeAppx.exe -ErrorAction SilentlyContinue |
      Where-Object { $_.FullName -match "\\x64\\MakeAppx\.exe$" }
  }

  $tool = $tools | Sort-Object FullName -Descending | Select-Object -First 1
  if ($null -eq $tool) {
    throw "MakeAppx.exe was not found. Install the Windows SDK with MSIX packaging tools."
  }
  return $tool.FullName
}

function Get-TargetOutputRoot {
  param([string]$Triple)

  Push-Location src-tauri
  try {
    $metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
  } finally {
    Pop-Location
  }

  return Join-Path $metadata.target_directory "$Triple\release"
}

function Copy-PackageAssets {
  param([string]$Destination)

  $assetDir = Join-Path $Destination "Assets"
  New-Item -ItemType Directory -Path $assetDir -Force | Out-Null

  foreach ($asset in @(
    "Square44x44Logo.png",
    "Square150x150Logo.png",
    "StoreLogo.png"
  )) {
    $source = Join-Path "src-tauri\icons" $asset
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
      throw "Required MSIX logo asset is missing: $source"
    }
    Copy-Item -LiteralPath $source -Destination (Join-Path $assetDir $asset) -Force
  }
}

function Write-AppxManifest {
  param(
    [string]$Destination,
    [string]$Architecture,
    [string]$Version,
    [string]$PackageName,
    [string]$Publisher,
    [string]$PublisherDisplayName
  )

  $manifest = @"
<?xml version="1.0" encoding="utf-8"?>
<Package
  xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
  IgnorableNamespaces="uap rescap">
  <Identity
    Name="$PackageName"
    Publisher="$Publisher"
    Version="$Version"
    ProcessorArchitecture="$Architecture" />
  <Properties>
    <DisplayName>Open Mind AI</DisplayName>
    <PublisherDisplayName>$PublisherDisplayName</PublisherDisplayName>
    <Logo>Assets\StoreLogo.png</Logo>
  </Properties>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.17763.0" MaxVersionTested="10.0.26100.0" />
  </Dependencies>
  <Resources>
    <Resource Language="en-us" />
  </Resources>
  <Applications>
    <Application Id="OpenMindAI" Executable="open-mind-ai.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements
        DisplayName="Open Mind AI"
        Description="Open Mind AI"
        BackgroundColor="transparent"
        Square44x44Logo="Assets\Square44x44Logo.png"
        Square150x150Logo="Assets\Square150x150Logo.png" />
    </Application>
  </Applications>
  <Capabilities>
    <rescap:Capability Name="runFullTrust" />
  </Capabilities>
</Package>
"@

  Set-Content -LiteralPath (Join-Path $Destination "AppxManifest.xml") -Value $manifest -Encoding utf8
}

function Assert-UnderDirectory {
  param([string]$Path, [string]$Root)

  $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
  $pathFull = [IO.Path]::GetFullPath($Path)
  if (-not $pathFull.StartsWith($rootFull, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Path must stay under $rootFull but got $pathFull"
  }
}

function Remove-DirectoryUnder {
  param([string]$Path, [string]$Root)

  Assert-UnderDirectory -Path $Path -Root $Root
  if (Test-Path -LiteralPath $Path -PathType Container) {
    Remove-Item -LiteralPath $Path -Recurse -Force
  }
}

function Remove-FileUnder {
  param([string]$Path, [string]$Root)

  Assert-UnderDirectory -Path $Path -Root $Root
  if (Test-Path -LiteralPath $Path -PathType Leaf) {
    Remove-Item -LiteralPath $Path -Force
  }
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Push-Location $repoRoot
try {
  $makeAppx = Get-MakeAppx
  $packageJson = Get-Content -LiteralPath "package.json" -Raw | ConvertFrom-Json
  $version = "$($packageJson.version).0"
  if ($version -notmatch "^\d+\.\d+\.\d+\.\d+$") {
    throw "MSIX version must be four numeric parts, got: $version"
  }

  $targets = @(
    @{ Triple = "x86_64-pc-windows-msvc"; Arch = "x64"; MsixArch = "x64" },
    @{ Triple = "i686-pc-windows-msvc"; Arch = "x86"; MsixArch = "x86" }
  )

  if (-not $SkipBuild) {
    foreach ($target in $targets) {
      npm run tauri -- build --target $target.Triple --config src-tauri/tauri.microsoftstore-all.conf.json --no-bundle
      if ($LASTEXITCODE -ne 0) {
        throw "Tauri app build failed for $($target.Triple) with exit code $LASTEXITCODE"
      }
    }
  }

  $stageRoot = Join-Path $repoRoot $OutputDir
  New-Item -ItemType Directory -Path $stageRoot -Force | Out-Null

  $packages = foreach ($target in $targets) {
    $targetRoot = Get-TargetOutputRoot -Triple $target.Triple
    $appExe = Join-Path $targetRoot "open-mind-ai.exe"
    if (-not (Test-Path -LiteralPath $appExe -PathType Leaf)) {
      throw "Built application executable was not found: $appExe"
    }

    $packageRoot = Join-Path $stageRoot "package-$($target.Arch)"
    Remove-DirectoryUnder -Path $packageRoot -Root $stageRoot
    New-Item -ItemType Directory -Path $packageRoot -Force | Out-Null

    Copy-Item -LiteralPath $appExe -Destination (Join-Path $packageRoot "open-mind-ai.exe") -Force
    $resources = Join-Path $targetRoot "resources"
    if (Test-Path -LiteralPath $resources -PathType Container) {
      Copy-Item -LiteralPath $resources -Destination (Join-Path $packageRoot "resources") -Recurse -Force
    }

    Copy-PackageAssets -Destination $packageRoot
    Write-AppxManifest `
      -Destination $packageRoot `
      -Architecture $target.MsixArch `
      -Version $version `
      -PackageName $PackageName `
      -Publisher $Publisher `
      -PublisherDisplayName $PublisherDisplayName

    $packagePath = Join-Path $stageRoot "OpenMindAI_$($packageJson.version)_$($target.Arch).msix"
    Remove-FileUnder -Path $packagePath -Root $stageRoot

    $makeAppxOutput = & $makeAppx pack /d $packageRoot /p $packagePath /o 2>&1
    $makeAppxOutput | ForEach-Object { Write-Host $_ }
    if ($LASTEXITCODE -ne 0) {
      throw "MakeAppx failed for $($target.Arch) with exit code $LASTEXITCODE"
    }

    Get-Item -LiteralPath $packagePath
  }

  $bundleInput = Join-Path $stageRoot "bundle-input"
  Remove-DirectoryUnder -Path $bundleInput -Root $stageRoot
  New-Item -ItemType Directory -Path $bundleInput -Force | Out-Null
  foreach ($package in $packages) {
    Copy-Item -LiteralPath $package.FullName -Destination (Join-Path $bundleInput $package.Name) -Force
  }

  $bundlePath = Join-Path $stageRoot "OpenMindAI_$($packageJson.version)_x86_x64.msixbundle"
  Remove-FileUnder -Path $bundlePath -Root $stageRoot
  $bundleOutput = & $makeAppx bundle /d $bundleInput /p $bundlePath /o 2>&1
  $bundleOutput | ForEach-Object { Write-Host $_ }
  if ($LASTEXITCODE -ne 0) {
    throw "MakeAppx bundle failed with exit code $LASTEXITCODE"
  }

  $allArtifacts = @($packages) + @(Get-Item -LiteralPath $bundlePath)
  $checksumLines = $allArtifacts | Sort-Object Name | ForEach-Object {
    $hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $($_.Name)"
  }
  Set-Content -LiteralPath (Join-Path $stageRoot "SHA256SUMS.txt") -Value $checksumLines -Encoding ascii

  Write-Host ""
  Write-Host "Microsoft Store MSIX packages are ready for Partner Center upload:" -ForegroundColor Green
  foreach ($artifact in $allArtifacts | Sort-Object Name) {
    Write-Host "  $($artifact.FullName)"
  }
  Write-Host "  $(Join-Path $stageRoot "SHA256SUMS.txt")"
  Write-Host ""
  Write-Host "If Partner Center reports an identity mismatch, use the Package/Identity Name and Publisher from the MSIX product page and re-run with -PackageName and -Publisher."
} finally {
  Pop-Location
}
