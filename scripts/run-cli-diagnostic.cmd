@echo off
setlocal
if "%~1"=="" (
  echo Drag and drop a PDF file onto this file.
  pause
  exit /b 2
)
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0_internal\run-cli.ps1" -InputPdf "%~f1" -NoPause
set "PDFX1A_CONVERT_EXIT=%ERRORLEVEL%"
echo.
echo Exit code: %PDFX1A_CONVERT_EXIT%
echo Review the output above.
pause
exit /b %PDFX1A_CONVERT_EXIT%
