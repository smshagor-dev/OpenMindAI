param(
  [Parameter(Mandatory = $true)]
  [string]$PfxPath,

  [Parameter(Mandatory = $true)]
  [string]$PfxPassword
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $PfxPath -PathType Leaf)) {
  throw "PFX file does not exist: $PfxPath"
}

$securePassword = ConvertTo-SecureString $PfxPassword -AsPlainText -Force
$certificate = Import-PfxCertificate `
  -FilePath $PfxPath `
  -CertStoreLocation Cert:\CurrentUser\My `
  -Password $securePassword

if ($null -eq $certificate) {
  throw "Certificate import failed."
}

$codeSigningCertificate = Get-ChildItem "Cert:\CurrentUser\My\$($certificate.Thumbprint)" -CodeSigningCert -ErrorAction SilentlyContinue
if ($null -eq $codeSigningCertificate) {
  throw "The imported certificate is not a code-signing certificate. Thumbprint=$($certificate.Thumbprint)"
}

if ($codeSigningCertificate.NotAfter -le (Get-Date)) {
  throw "The imported code-signing certificate is expired. Thumbprint=$($certificate.Thumbprint)"
}

Write-Host ""
Write-Host "Code-signing certificate installed:" -ForegroundColor Green
Write-Host "  Subject:    $($codeSigningCertificate.Subject)"
Write-Host "  Thumbprint: $($codeSigningCertificate.Thumbprint)"
Write-Host "  Expires:    $($codeSigningCertificate.NotAfter)"
Write-Host ""
Write-Host "Build Microsoft Store EXEs with:"
Write-Host "  npm run build:microsoft-store -- -CertificateThumbprint $($codeSigningCertificate.Thumbprint)"
