param(
  [string]$CertificateThumbprint = $env:WINDOWS_CERTIFICATE_THUMBPRINT,
  [string]$PfxPath = $env:WINDOWS_CERTIFICATE_PFX_PATH,
  [string]$PfxPassword = $env:WINDOWS_CERTIFICATE_PASSWORD,
  [string]$TimestampUrl = "http://timestamp.digicert.com",
  [string]$OutputDir = "release-output\microsoftstore-exe",
  [switch]$SkipSmokeInstall
)

$ErrorActionPreference = "Stop"
$signingConfigPath = $null

function Find-CodeSigningCertificate {
  param([string]$Thumbprint)

  if ([string]::IsNullOrWhiteSpace($Thumbprint)) {
    $certs = @(Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert |
      Where-Object { $_.NotAfter -gt (Get-Date) } |
      Sort-Object NotAfter -Descending)

    if ($certs.Count -eq 1) {
      return $certs[0]
    }

    if ($certs.Count -gt 1) {
      $list = $certs | ForEach-Object { "  $($_.Thumbprint)  $($_.Subject)  expires=$($_.NotAfter)" }
      throw "Multiple code signing certificates were found. Re-run with -CertificateThumbprint.`n$($list -join "`n")"
    }

    return $null
  }

  $normalized = $Thumbprint -replace "\s", ""
  $cert = Get-ChildItem "Cert:\CurrentUser\My\$normalized" -CodeSigningCert -ErrorAction SilentlyContinue
  if ($null -eq $cert) {
    throw "Code signing certificate not found in CurrentUser\My: $Thumbprint"
  }
  if ($cert.NotAfter -le (Get-Date)) {
    throw "Code signing certificate has expired: $Thumbprint"
  }
  return $cert
}

function Import-CodeSigningPfx {
  param([string]$Path, [string]$Password)

  if ([string]::IsNullOrWhiteSpace($Path)) {
    return $null
  }
  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    throw "PFX file does not exist: $Path"
  }
  if ([string]::IsNullOrWhiteSpace($Password)) {
    throw "PFX password is required. Set WINDOWS_CERTIFICATE_PASSWORD or pass -PfxPassword."
  }

  $securePassword = ConvertTo-SecureString $Password -AsPlainText -Force
  $cert = Import-PfxCertificate -FilePath $Path -CertStoreLocation Cert:\CurrentUser\My -Password $securePassword
  if ($null -eq $cert) {
    throw "The PFX certificate could not be imported."
  }
  return $cert
}

function Assert-ValidAuthenticode {
  param([string]$Path)

  $signature = Get-AuthenticodeSignature -FilePath $Path
  if ($signature.Status -ne "Valid") {
    throw "Authenticode verification failed for $Path. Status=$($signature.Status) Message=$($signature.StatusMessage)"
  }
  return $signature
}

function Get-TargetOutputRoot {
  param([string]$Triple)

  Push-Location src-tauri
  try {
    $metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
  } finally {
    Pop-Location
  }

  return Join-Path $metadata.target_directory "$Triple\release\bundle\nsis"
}

function Invoke-SilentSmokeInstall {
  param([string]$InstallerPath, [string]$Arch)

  $tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
  $root = Join-Path $tempBase "openmindai-store-smoke-$Arch"
  $rootFull = [IO.Path]::GetFullPath($root)
  if (-not $rootFull.StartsWith($tempBase, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Smoke install path must stay under the temp directory: $rootFull"
  }

  if (Test-Path -LiteralPath $root) {
    Remove-Item -LiteralPath $root -Recurse -Force
  }
  New-Item -ItemType Directory -Path $root -Force | Out-Null

  try {
    $install = Start-Process -FilePath $InstallerPath -ArgumentList @("/S", "/D=$root") -Wait -PassThru -WindowStyle Hidden
    if ($install.ExitCode -ne 0) {
      throw "Silent install failed for $InstallerPath with exit code $($install.ExitCode)"
    }

    $peFiles = @(Get-ChildItem -LiteralPath $root -Recurse -File |
      Where-Object { $_.Extension -in ".exe", ".dll" })
    if ($peFiles.Count -eq 0) {
      throw "Silent install produced no PE files under $root"
    }

    foreach ($file in $peFiles) {
      Assert-ValidAuthenticode -Path $file.FullName | Out-Null
    }
  } finally {
    $uninstaller = Get-ChildItem -LiteralPath $root -Recurse -File -Filter "*.exe" -ErrorAction SilentlyContinue |
      Where-Object { $_.Name -match "(?i)uninstall" } |
      Select-Object -First 1

    if ($null -ne $uninstaller) {
      Start-Process -FilePath $uninstaller.FullName -ArgumentList "/S" -Wait -WindowStyle Hidden | Out-Null
    }
    if (Test-Path -LiteralPath $root) {
      $rootFull = [IO.Path]::GetFullPath($root)
      if (-not $rootFull.StartsWith($tempBase, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Smoke cleanup path must stay under the temp directory: $rootFull"
      }
      Remove-Item -LiteralPath $root -Recurse -Force
    }
  }
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Push-Location $repoRoot
try {
  $importedCert = Import-CodeSigningPfx -Path $PfxPath -Password $PfxPassword
  if ($null -ne $importedCert) {
    $CertificateThumbprint = $importedCert.Thumbprint
  }

  $certificate = Find-CodeSigningCertificate -Thumbprint $CertificateThumbprint
  if ($null -eq $certificate) {
    throw @"
No trusted code-signing certificate is installed.

Microsoft Store Win32 EXE submissions require Authenticode signing with a CA-trusted certificate.
Install/import a real code-signing PFX first, then run one of:
  .\scripts\build-microsoft-store-exe.ps1 -PfxPath C:\path\cert.pfx -PfxPassword "password"
  .\scripts\build-microsoft-store-exe.ps1 -CertificateThumbprint THUMBPRINT
"@
  }

  $script:signingConfigPath = Join-Path $repoRoot "src-tauri\tauri.microsoftstore-signing.local.conf.json"
  $signingConfig = @{
    bundle = @{
      windows = @{
        certificateThumbprint = $certificate.Thumbprint
        digestAlgorithm       = "sha256"
        timestampUrl          = $TimestampUrl
      }
    }
  } | ConvertTo-Json -Depth 10
  Set-Content -LiteralPath $script:signingConfigPath -Value $signingConfig -Encoding utf8

  $targets = @(
    @{ Triple = "x86_64-pc-windows-msvc"; Arch = "x64" },
    @{ Triple = "i686-pc-windows-msvc"; Arch = "x86" }
  )

  foreach ($target in $targets) {
    npm run tauri -- build --target $target.Triple --config src-tauri/tauri.microsoftstore-all.conf.json --config $script:signingConfigPath
    if ($LASTEXITCODE -ne 0) {
      throw "Tauri build failed for $($target.Triple) with exit code $LASTEXITCODE"
    }
  }

  $stageDir = Join-Path $repoRoot $OutputDir
  New-Item -ItemType Directory -Path $stageDir -Force | Out-Null

  $staged = foreach ($target in $targets) {
    $bundleDir = Get-TargetOutputRoot -Triple $target.Triple
    $installer = Get-ChildItem -LiteralPath $bundleDir -Filter "OpenMindAI_*_$($target.Arch)-setup.exe" -File |
      Sort-Object LastWriteTime -Descending |
      Select-Object -First 1
    if ($null -eq $installer) {
      throw "Installer was not found for $($target.Triple) under $bundleDir"
    }

    $destination = Join-Path $stageDir $installer.Name
    Copy-Item -LiteralPath $installer.FullName -Destination $destination -Force
    Assert-ValidAuthenticode -Path $destination | Out-Null

    if (-not $SkipSmokeInstall) {
      Invoke-SilentSmokeInstall -InstallerPath $destination -Arch $target.Arch
    }

    Get-Item -LiteralPath $destination
  }

  $checksumLines = $staged | Sort-Object Name | ForEach-Object {
    $hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $($_.Name)"
  }
  Set-Content -LiteralPath (Join-Path $stageDir "SHA256SUMS.txt") -Value $checksumLines -Encoding ascii

  Write-Host ""
  Write-Host "Microsoft Store EXE artifacts are signed and ready:" -ForegroundColor Green
  foreach ($file in $staged | Sort-Object Name) {
    Write-Host "  $($file.FullName)"
  }
  Write-Host "  $(Join-Path $stageDir "SHA256SUMS.txt")"
} finally {
  if (-not [string]::IsNullOrWhiteSpace($script:signingConfigPath) -and (Test-Path -LiteralPath $script:signingConfigPath)) {
    Remove-Item -LiteralPath $script:signingConfigPath -Force
  }
  Pop-Location
}
