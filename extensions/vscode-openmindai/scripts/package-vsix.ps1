param(
  [string]$OutputPath
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$packageJsonPath = Join-Path $root "package.json"
$packageJson = Get-Content -Raw $packageJsonPath | ConvertFrom-Json

if (-not $OutputPath) {
  $OutputPath = Join-Path $root ("dist/{0}-{1}.vsix" -f $packageJson.name, $packageJson.version)
}

$dist = Join-Path $root "dist"
$stageRoot = Join-Path $root ".vsix-stage"
$extensionRoot = Join-Path $stageRoot "extension"

if (-not (Test-Path (Join-Path $dist "extension.js"))) {
  throw "dist/extension.js is missing. Run npm run compile first."
}

Remove-Item -Recurse -Force $stageRoot -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $extensionRoot | Out-Null
New-Item -ItemType Directory -Force (Split-Path -Parent $OutputPath) | Out-Null

foreach ($relative in @("package.json", "README.md", ".vscodeignore")) {
  Copy-Item -LiteralPath (Join-Path $root $relative) -Destination (Join-Path $extensionRoot $relative)
}

foreach ($folder in @("dist", "media")) {
  Copy-Item -Recurse -LiteralPath (Join-Path $root $folder) -Destination (Join-Path $extensionRoot $folder)
}

Get-ChildItem -Recurse -LiteralPath (Join-Path $extensionRoot "dist") -Filter "*.map" | Remove-Item -Force
# Integration-test harness (loaded only in VS Code's extension Test mode); never shipped.
Remove-Item -LiteralPath (Join-Path $extensionRoot "dist/testHooks.js") -Force -ErrorAction SilentlyContinue
Get-ChildItem -LiteralPath (Join-Path $extensionRoot "dist") -Filter "*.vsix" -ErrorAction SilentlyContinue | Remove-Item -Force

$contentTypes = @'
<?xml version="1.0" encoding="utf-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="json" ContentType="application/json"/>
  <Default Extension="js" ContentType="application/javascript"/>
  <Default Extension="md" ContentType="text/markdown"/>
  <Default Extension="png" ContentType="image/png"/>
  <Default Extension="svg" ContentType="image/svg+xml"/>
  <Default Extension="txt" ContentType="text/plain"/>
  <Default Extension="vsixmanifest" ContentType="text/xml"/>
</Types>
'@

$description = [System.Security.SecurityElement]::Escape($packageJson.description)
$displayName = [System.Security.SecurityElement]::Escape($packageJson.displayName)
$tags = [System.Security.SecurityElement]::Escape(($packageJson.keywords -join ","))
$categories = [System.Security.SecurityElement]::Escape(($packageJson.categories -join ","))
$engine = [System.Security.SecurityElement]::Escape($packageJson.engines.vscode)

$manifest = @"
<?xml version="1.0" encoding="utf-8"?>
<PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011">
  <Metadata>
    <Identity Language="en-US" Id="$($packageJson.name)" Version="$($packageJson.version)" Publisher="$($packageJson.publisher)" />
    <DisplayName>$displayName</DisplayName>
    <Description xml:space="preserve">$description</Description>
    <Tags>$tags</Tags>
    <Categories>$categories</Categories>
    <Properties>
      <Property Id="Microsoft.VisualStudio.Code.Engine" Value="$engine" />
    </Properties>
  </Metadata>
  <Installation>
    <InstallationTarget Id="Microsoft.VisualStudio.Code"/>
  </Installation>
  <Dependencies/>
  <Assets>
    <Asset Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json" Addressable="true" />
    <Asset Type="Microsoft.VisualStudio.Services.Content.Details" Path="extension/README.md" Addressable="true" />
    <Asset Type="Microsoft.VisualStudio.Services.Icons.Default" Path="extension/media/icon.png" Addressable="true" />
  </Assets>
</PackageManifest>
"@

Set-Content -LiteralPath (Join-Path $stageRoot "[Content_Types].xml") -Value $contentTypes -Encoding UTF8
Set-Content -LiteralPath (Join-Path $stageRoot "extension.vsixmanifest") -Value $manifest -Encoding UTF8

$tempZip = "$OutputPath.zip"
Remove-Item -Force $OutputPath -ErrorAction SilentlyContinue
Remove-Item -Force $tempZip -ErrorAction SilentlyContinue
Compress-Archive -Path (Join-Path $stageRoot "*") -DestinationPath $tempZip -Force
Move-Item -LiteralPath $tempZip -Destination $OutputPath
Remove-Item -Recurse -Force $stageRoot

Write-Host "Created $OutputPath"
