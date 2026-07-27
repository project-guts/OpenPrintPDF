param(
    [Parameter(Mandatory = $true)]
    [string]$InputPdf,
    [string]$DiagnosticsRoot,
    [switch]$NoPause
)

$ErrorActionPreference = "Continue"
$internalRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$cli = Join-Path $internalRoot "pdfx1a-convert.exe"
$ghostscript = Join-Path $internalRoot "ghostscript/bin/gswin64c.exe"

function Wait-BeforeExit {
    if (-not $NoPause) {
        Read-Host "Press Enter to close"
    }
}

if (-not (Test-Path -LiteralPath $InputPdf -PathType Leaf)) {
    Write-Host "Input PDF was not found: $InputPdf" -ForegroundColor Red
    Wait-BeforeExit
    exit 2
}
if ([IO.Path]::GetExtension($InputPdf) -ine ".pdf") {
    Write-Host "Please specify a PDF file: $InputPdf" -ForegroundColor Red
    Wait-BeforeExit
    exit 2
}
if (-not (Test-Path -LiteralPath $cli -PathType Leaf)) {
    Write-Host "CLI executable was not found: $cli" -ForegroundColor Red
    Wait-BeforeExit
    exit 2
}
if (-not (Test-Path -LiteralPath $ghostscript -PathType Leaf)) {
    Write-Host "Bundled Ghostscript was not found: $ghostscript" -ForegroundColor Red
    Wait-BeforeExit
    exit 2
}

$resolvedInput = [IO.Path]::GetFullPath($InputPdf)
$directory = Split-Path -Parent $resolvedInput
$stem = [IO.Path]::GetFileNameWithoutExtension($resolvedInput)
$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$finalOutput = Join-Path $directory "$stem-pdfx1a.pdf"
if (Test-Path -LiteralPath $finalOutput) {
    $finalOutput = Join-Path $directory "$stem-pdfx1a-$timestamp.pdf"
}
if ([string]::IsNullOrWhiteSpace($DiagnosticsRoot)) {
    if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        $DiagnosticsRoot = Join-Path $env:LOCALAPPDATA "pdfx1a-convert/diagnostics"
    } else {
        $DiagnosticsRoot = Join-Path ([IO.Path]::GetTempPath()) "pdfx1a-convert/diagnostics"
    }
}
$runId = "$timestamp-$([Guid]::NewGuid().ToString('N').Substring(0, 8))"
$runDirectory = Join-Path $DiagnosticsRoot $runId
New-Item -ItemType Directory -Force $runDirectory | Out-Null
$workingOutput = Join-Path $runDirectory "output.pdf"
$report = Join-Path $runDirectory "report.json"
$log = Join-Path $runDirectory "run.log"
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)

$header = [string[]]@(
    "pdfx1a-convert diagnostic"
    "Started: $(Get-Date -Format o)"
    "Input: $resolvedInput"
    "Working output: $workingOutput"
    "Final output: $finalOutput"
    "CLI: $cli"
    "Ghostscript: $ghostscript"
    "OS: $([Environment]::OSVersion.VersionString)"
    "64-bit OS: $([Environment]::Is64BitOperatingSystem)"
    "Processor: $env:PROCESSOR_IDENTIFIER"
    "Logical processors: $env:NUMBER_OF_PROCESSORS"
    ""
)
[IO.File]::WriteAllLines($log, $header, $utf8NoBom)

function Write-LogLine {
    param([string]$Line)
    [IO.File]::AppendAllText($log, "$Line`r`n", $utf8NoBom)
}

function Invoke-Logged {
    param([string]$Label, [string[]]$Arguments)
    $heading = "===== $Label ====="
    Write-Host $heading
    Write-LogLine $heading
    $commandOutput = @(& $cli @Arguments 2>&1)
    $code = $LASTEXITCODE
    foreach ($line in $commandOutput) {
        $text = [string]$line
        Write-LogLine $text
        if ($code -ne 0) {
            Write-Host $text
        }
    }
    $exitLine = "Exit code: $code"
    Write-LogLine $exitLine
    Write-LogLine ""
    if ($code -eq 0) {
        Write-Host "OK" -ForegroundColor Green
    } else {
        Write-Host $exitLine -ForegroundColor Red
    }
    return $code
}

Invoke-Logged "CLI version" @("--version") | Out-Null
Invoke-Logged "Ghostscript engine" @(
    "engine-info",
    "--ghostscript", $ghostscript
) | Out-Null
Invoke-Logged "Input inspection" @(
    "inspect", $resolvedInput, "--json"
) | Out-Null
$conversionExitCode = Invoke-Logged "PDF/X-1a conversion" @(
    "convert-pdfx1a", $resolvedInput,
    "--output", $workingOutput,
    "--ghostscript", $ghostscript,
    "--report", $report
)

Write-Host ""
if ($conversionExitCode -eq 0) {
    try {
        Move-Item -LiteralPath $workingOutput -Destination $finalOutput -ErrorAction Stop
        $statusLine = "Conversion succeeded."
        $outputLine = "Output PDF: $finalOutput"
        Write-Host $statusLine -ForegroundColor Green
        Write-Host $outputLine
        Write-LogLine $statusLine
        Write-LogLine $outputLine
    } catch {
        $conversionExitCode = 1
        $statusLine = "Conversion succeeded, but the final PDF could not be moved."
        $moveErrorLine = [string]$_.Exception.Message
        Write-Host $statusLine -ForegroundColor Red
        Write-Host $moveErrorLine
        Write-LogLine $statusLine
        Write-LogLine $moveErrorLine
    }
} else {
    $statusLine = "Conversion failed."
    $diagnosticLine = "Diagnostic files: $runDirectory"
    Write-Host $statusLine -ForegroundColor Red
    Write-Host $diagnosticLine
    Write-LogLine $statusLine
    Write-LogLine $diagnosticLine
}
if ($conversionExitCode -ne 0) {
    $logLine = "Log: $log"
    Write-Host $logLine
    Write-LogLine $logLine
}
Write-Host ""
Wait-BeforeExit
exit $conversionExitCode
