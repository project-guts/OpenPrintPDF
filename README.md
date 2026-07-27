# pdfx1a-convert

Windows utility for PDF inspection and PDF/X-1a conversion.

Status: pre-release.

The portable Windows package keeps a simple drag-and-drop interface. Extract the
ZIP and drop a PDF onto `ここにPDFをドラッグ＆ドロップ.cmd`. The converted file
is saved beside the input as `original-name-pdfx1a.pdf`.

Ghostscript is bundled. Logs, JSON reports, and diagnostic intermediate PDFs are
stored under `%LOCALAPPDATA%\pdfx1a-convert\diagnostics` instead of beside the
user's PDF. Supporting executables and license documents are kept in
`_internal`.

## Requirements

- Windows 11 x64
- Rust stable
- Node.js 24 or later
- Visual Studio 2022 Build Tools

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
