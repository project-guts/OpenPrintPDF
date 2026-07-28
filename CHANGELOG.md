# Changelog

## 0.1.11 — 2026-07-28

- Windowsドラッグ＆ドロップ版の起動処理と説明を簡素化

## 0.1.10 — 2026-07-28

- Windowsドラッグ＆ドロップ版の案内表示を改善

## 0.1.9 — 2026-07-28

- Added the portable Windows `pdfx1a-convert` CLI package.
- Added `ここにPDFをドラッグ＆ドロップ.cmd` as the user-facing launcher.
- Bundled Ghostscript 10.07.1 under `_internal`.
- Save only the converted PDF beside the input file.
- Store logs, JSON reports, and diagnostic PDFs under
  `%LOCALAPPDATA%\pdfx1a-convert\diagnostics`.
- Added SHA-256 verification for the distributed ZIP.

## 0.1.8 — 2026-07-26

- Initial public source release.
