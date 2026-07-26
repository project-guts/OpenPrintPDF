param(
    [string]$ManifestPath = "build/ghostscript-windows.json",
    [string]$StageDirectory = "apps/desktop/src-tauri/vendor/ghostscript",
    [string]$DownloadDirectory = "$env:RUNNER_TEMP/open-print-pdf-downloads",
    [switch]$DownloadSource
)

$ErrorActionPreference = "Stop"
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$manifestFile = [IO.Path]::GetFullPath((Join-Path $repoRoot $ManifestPath))
$stagePath = [IO.Path]::GetFullPath((Join-Path $repoRoot $StageDirectory))
$allowedStage = [IO.Path]::GetFullPath((Join-Path $repoRoot "apps/desktop/src-tauri/vendor/ghostscript"))

if ($stagePath -ne $allowedStage) {
    throw "Refusing to replace unexpected stage directory: $stagePath"
}

$manifest = Get-Content -Raw $manifestFile | ConvertFrom-Json
New-Item -ItemType Directory -Force $DownloadDirectory | Out-Null

function Get-VerifiedFile {
    param($Asset)
    $destination = Join-Path $DownloadDirectory $Asset.fileName
    if (-not (Test-Path $destination)) {
        Write-Host "Downloading $($Asset.url)"
        & curl.exe --location --fail --retry 3 --retry-all-errors --connect-timeout 30 --max-time 300 --output $destination $Asset.url
        if ($LASTEXITCODE -ne 0) {
            throw "Download failed for $($Asset.url) with curl exit code $LASTEXITCODE"
        }
    }
    Write-Host "Verifying SHA-256 for $($Asset.fileName)"
    $actual = (Get-FileHash -Algorithm SHA256 $destination).Hash.ToLowerInvariant()
    $expected = $Asset.sha256.ToLowerInvariant()
    if ($actual -ne $expected) {
        throw "SHA-256 mismatch for $($Asset.fileName): expected $expected, got $actual"
    }
    return $destination
}

$installer = Get-VerifiedFile $manifest.windowsX64
$installRoot = Join-Path $DownloadDirectory "ghostscript-install"
if (Test-Path $installRoot) {
    Remove-Item -Recurse -Force $installRoot
}
New-Item -ItemType Directory -Force $installRoot | Out-Null

$sevenZip = Join-Path $env:ProgramFiles "7-Zip/7z.exe"
if (-not (Test-Path $sevenZip)) {
    throw "7-Zip was not found at $sevenZip"
}
Write-Host "Extracting the verified Ghostscript installer with 7-Zip"
& $sevenZip x -y "-o$installRoot" $installer
if ($LASTEXITCODE -ne 0) {
    throw "Ghostscript extraction failed with 7-Zip exit code $LASTEXITCODE"
}

$consoleExe = Get-ChildItem -Path $installRoot -Recurse -Filter "gswin64c.exe" | Select-Object -First 1
if (-not $consoleExe) {
    throw "gswin64c.exe was not found after extracting Ghostscript"
}
$ghostscriptRoot = Split-Path (Split-Path $consoleExe.FullName -Parent) -Parent
Write-Host "Staging Ghostscript from $ghostscriptRoot"

if (Test-Path $stagePath) {
    Remove-Item -Recurse -Force $stagePath
}
New-Item -ItemType Directory -Force $stagePath | Out-Null
foreach ($directory in @("bin", "lib", "Resource", "fonts", "doc")) {
    $sourceDirectory = Join-Path $ghostscriptRoot $directory
    if (Test-Path $sourceDirectory) {
        Copy-Item -Recurse $sourceDirectory (Join-Path $stagePath $directory)
    }
}

$stagedConsole = Join-Path $stagePath "bin/gswin64c.exe"
if (-not (Test-Path $stagedConsole)) {
    throw "The staged Ghostscript console executable is missing: $stagedConsole"
}

$versionFile = Join-Path $stagePath "OPEN_PRINT_PDF_GHOSTSCRIPT_VERSION.txt"
@(
    "Ghostscript $($manifest.version)",
    "Windows binary SHA-256: $($manifest.windowsX64.sha256)",
    "Corresponding source SHA-256: $($manifest.source.sha256)",
    "Source: $($manifest.source.url)"
) | Set-Content -Encoding UTF8 $versionFile

$sourceArchive = ""
if ($DownloadSource) {
    $sourceArchive = Get-VerifiedFile $manifest.source
}

if ($env:GITHUB_OUTPUT) {
    "ghostscript_root=$stagePath" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8
    "ghostscript_exe=$(Join-Path $stagePath 'bin/gswin64c.exe')" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8
    "source_archive=$sourceArchive" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8
    "version=$($manifest.version)" | Out-File -FilePath $env:GITHUB_OUTPUT -Append -Encoding utf8
}

Write-Host "Prepared Ghostscript $($manifest.version) at $stagePath"
