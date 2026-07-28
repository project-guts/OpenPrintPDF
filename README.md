# pdfx1a-convert

Windows utility for PDF inspection and PDF/X-1a conversion.

Status: beta.

This is a small, independently published command-line utility. It is not a
hosted service and does not upload PDFs.

The portable Windows package keeps a simple drag-and-drop interface. Extract the
ZIP and drop a PDF onto `ここにPDFをドラッグ＆ドロップ.cmd`. The converted file
is saved beside the input as `original-name-pdfx1a.pdf`.

Ghostscript is bundled. Logs, JSON reports, and diagnostic intermediate PDFs are
stored under `%LOCALAPPDATA%\pdfx1a-convert\diagnostics` instead of beside the
user's PDF. Supporting executables and license documents are kept in
`_internal`.

## Requirements

- Windows 11 x64

The Windows ZIP already includes the executable and Ghostscript. Rust, Node.js,
and Visual Studio are needed only when building from source.

## Download

Download the ZIP and matching `.sha256` file from
[GitHub Releases](https://github.com/project-guts/pdfx1a-convert/releases).

## Build

```powershell
cargo test --workspace
cargo build -p open-print-pdf-cli --release
cd apps/desktop
npm ci
npm run build
```

## Documents

- [License](LICENSE)
- [Third-party notices](THIRD_PARTY_NOTICES.md)
- [Changelog](CHANGELOG.md)
- [Windows portable package usage](docs/windows-cli-diagnostic.txt)
