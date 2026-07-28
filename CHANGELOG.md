# Changelog

## 0.1.10 — 2026-07-28

- Parallels共有フォルダ、ネットワークドライブ、UNCパス上からの起動を
  事前に検出し、フォルダ一式をCドライブへ移動するよう案内する
- Windows配布物のREADMEにローカルドライブ要件を明記

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
