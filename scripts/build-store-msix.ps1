param(
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Release",
    [string]$PackageVersion = "",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$configPath = Join-Path $repoRoot "build/store-msix.json"
$templatePath = Join-Path $repoRoot "build/msix/AppxManifest.xml.template"
$targetRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "target/store-msix"))
$allowedTargetRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "target/store-msix"))

if ($targetRoot -ne $allowedTargetRoot) {
    throw "Refusing to use unexpected MSIX target directory: $targetRoot"
}
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw "Microsoft Store MSIX packages must be built on Windows"
}

$config = [IO.File]::ReadAllText($configPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
if ([string]::IsNullOrWhiteSpace($PackageVersion)) {
    $PackageVersion = $config.packageVersion
}
if ($PackageVersion -notmatch '^[1-9][0-9]{0,4}\.[0-9]{1,5}\.[0-9]{1,5}\.0$') {
    throw "PackageVersion must be four numeric parts, start above zero, and end in .0: $PackageVersion"
}
foreach ($part in $PackageVersion.Split(".")) {
    if ([int]$part -gt 65535) {
        throw "Each PackageVersion part must be 0 through 65535: $PackageVersion"
    }
}
if ($config.architecture -ne "x64") {
    throw "The initial Store package must target x64 only"
}
if ($config.minimumWindowsVersion -ne "10.0.22000.0") {
    throw "The initial Store package must require Windows 11 (10.0.22000.0)"
}

$profileDirectory = $Configuration.ToLowerInvariant()
$rustTarget = switch ($config.architecture) {
    "x64" { "x86_64-pc-windows-msvc" }
    default { throw "Unsupported Store architecture: $($config.architecture)" }
}
$executablePath = Join-Path $repoRoot "target/$rustTarget/$profileDirectory/$($config.executable)"
$ghostscriptPath = Join-Path $repoRoot "apps/desktop/src-tauri/vendor/ghostscript"
$ghostscriptExecutable = Join-Path $ghostscriptPath "bin/gswin64c.exe"

if (-not $SkipBuild) {
    Push-Location (Join-Path $repoRoot "apps/desktop")
    try {
        & npm ci
        if ($LASTEXITCODE -ne 0) {
            throw "npm ci failed with exit code $LASTEXITCODE"
        }
        & npm run build
        if ($LASTEXITCODE -ne 0) {
            throw "frontend build failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        Pop-Location
    }

    $cargoArguments = @(
        "build",
        "-p", "open-print-pdf-desktop",
        "--target", $rustTarget
    )
    if ($Configuration -eq "Release") {
        $cargoArguments += "--release"
    }
    & cargo @cargoArguments
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
}

if (-not (Test-Path $executablePath -PathType Leaf)) {
    throw "Desktop executable is missing: $executablePath"
}
if (-not (Test-Path $ghostscriptExecutable -PathType Leaf)) {
    throw "Bundled Ghostscript is missing. Run scripts/prepare-ghostscript-windows.ps1 first."
}

$windowsKitsRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits/10/bin"
$makeAppx = Get-ChildItem -Path $windowsKitsRoot -Recurse -Filter "makeappx.exe" |
    Where-Object { $_.FullName -match '[\\/]x64[\\/]makeappx\.exe$' } |
    Sort-Object { [version]$_.Directory.Parent.Name } -Descending |
    Select-Object -First 1
if (-not $makeAppx) {
    throw "MakeAppx.exe was not found. Install the Windows SDK."
}

if (Test-Path $targetRoot) {
    Remove-Item -Recurse -Force $targetRoot
}
$stageDirectory = Join-Path $targetRoot "staging"
$outputDirectory = Join-Path $targetRoot "output"
$uploadDirectory = Join-Path $targetRoot "upload"
$verificationDirectory = Join-Path $targetRoot "verification"
New-Item -ItemType Directory -Force -Path @(
    $stageDirectory,
    $outputDirectory,
    $uploadDirectory
) | Out-Null

Copy-Item $executablePath (Join-Path $stageDirectory $config.executable)
Copy-Item -Recurse $ghostscriptPath (Join-Path $stageDirectory "ghostscript")

$licenseDirectory = Join-Path $stageDirectory "licenses"
New-Item -ItemType Directory -Force $licenseDirectory | Out-Null
Copy-Item (Join-Path $repoRoot "LICENSE") (Join-Path $licenseDirectory "OpenPrintPDF-AGPL-3.0.txt")
Copy-Item (Join-Path $repoRoot "THIRD_PARTY_NOTICES.md") $licenseDirectory

$assetDirectory = Join-Path $stageDirectory "Assets"
New-Item -ItemType Directory -Force $assetDirectory | Out-Null
$iconDirectory = Join-Path $repoRoot "apps/desktop/src-tauri/icons"
foreach ($asset in @(
    "StoreLogo.png",
    "Square44x44Logo.png",
    "Square150x150Logo.png"
)) {
    Copy-Item (Join-Path $iconDirectory $asset) (Join-Path $assetDirectory $asset)
}

function ConvertTo-XmlText {
    param([string]$Value)
    return [Security.SecurityElement]::Escape($Value)
}

$manifest = [IO.File]::ReadAllText($templatePath, [Text.Encoding]::UTF8)
$replacements = [ordered]@{
    "{{IDENTITY_NAME}}" = $config.identityName
    "{{PUBLISHER}}" = $config.publisher
    "{{PACKAGE_VERSION}}" = $PackageVersion
    "{{ARCHITECTURE}}" = $config.architecture
    "{{DISPLAY_NAME}}" = $config.displayName
    "{{PUBLISHER_DISPLAY_NAME}}" = $config.publisherDisplayName
    "{{DESCRIPTION}}" = $config.description
    "{{MINIMUM_WINDOWS_VERSION}}" = $config.minimumWindowsVersion
    "{{MAXIMUM_WINDOWS_VERSION_TESTED}}" = $config.maximumWindowsVersionTested
    "{{EXECUTABLE}}" = $config.executable
}
foreach ($replacement in $replacements.GetEnumerator()) {
    $manifest = $manifest.Replace($replacement.Key, (ConvertTo-XmlText $replacement.Value))
}
if ($manifest -match '\{\{[A-Z_]+\}\}') {
    throw "The AppxManifest template contains an unresolved placeholder"
}
$manifestPath = Join-Path $stageDirectory "AppxManifest.xml"
$manifest | Set-Content -Encoding utf8 $manifestPath

$packageName = "OpenPrintPDF_${PackageVersion}_x64"
$msixPath = Join-Path $outputDirectory "$packageName.msix"
& $makeAppx.FullName pack /v /o /d $stageDirectory /p $msixPath
if ($LASTEXITCODE -ne 0) {
    throw "MakeAppx pack failed with exit code $LASTEXITCODE"
}

& $makeAppx.FullName unpack /o /p $msixPath /d $verificationDirectory
if ($LASTEXITCODE -ne 0) {
    throw "MakeAppx could not unpack the generated MSIX"
}
foreach ($requiredPath in @(
    "AppxManifest.xml",
    $config.executable,
    "ghostscript/bin/gswin64c.exe",
    "licenses/OpenPrintPDF-AGPL-3.0.txt",
    "licenses/THIRD_PARTY_NOTICES.md"
)) {
    if (-not (Test-Path (Join-Path $verificationDirectory $requiredPath) -PathType Leaf)) {
        throw "Generated MSIX is missing required file: $requiredPath"
    }
}

Copy-Item $msixPath (Join-Path $uploadDirectory (Split-Path $msixPath -Leaf))
$zipPath = Join-Path $targetRoot "$packageName.zip"
$msixUploadPath = Join-Path $outputDirectory "$packageName.msixupload"
Compress-Archive -Path (Join-Path $uploadDirectory "*") -DestinationPath $zipPath -CompressionLevel Optimal
Move-Item $zipPath $msixUploadPath

$checksumPath = Join-Path $outputDirectory "SHA256SUMS.txt"
$checksumLines = foreach ($artifact in @($msixPath, $msixUploadPath)) {
    $hash = (Get-FileHash -Algorithm SHA256 $artifact).Hash.ToLowerInvariant()
    "$hash  $(Split-Path $artifact -Leaf)"
}
$checksumLines | Set-Content -Encoding ascii $checksumPath

if ($env:GITHUB_OUTPUT) {
    "msix=$msixPath" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8
    "msixupload=$msixUploadPath" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8
    "checksums=$checksumPath" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8
    "package_version=$PackageVersion" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8
}

Write-Host "Created Microsoft Store package:"
Write-Host "  $msixUploadPath"
Write-Host "Upload the .msixupload file to Partner Center. Do not distribute the unsigned .msix directly."
