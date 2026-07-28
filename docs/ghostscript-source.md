# 同梱Ghostscriptのソースコード

pdfx1a-convertのWindowsポータブルZIPには、Ghostscript 10.07.1のWindows
x64版を同梱する。

## 対応するソースコード

Windows ZIPを取得したpdfx1a-convertのGitHub Releaseに、次のファイルを
同時掲載する。

```text
ghostscript-10.07.1.tar.xz
SHA-256: 1cdb766de8db8f1e589c817f09c5855ea5f65dfc8540e465a69ac14c18416025
```

同じソースはArtifex Softwareの公式リリースからも取得できる。

https://github.com/ArtifexSoftware/ghostpdl-downloads/releases/download/gs10071/ghostscript-10.07.1.tar.xz

Windowsバイナリは次の公式配布物から取り出し、pdfx1a-convertのリソースとして
同梱する。

```text
gs10071w64.exe
SHA-256: 3a4c28d0aac47aa7cccd35a5932c55110376e9dbd966898dde388b7faba444a4
```

## 再現方法

`build/ghostscript-windows.json`がバージョン、公式URL、SHA-256を固定している。
Windows上で次を実行すると、ハッシュを検証したうえでポータブルZIP用の
リソースを準備する。

```powershell
./scripts/prepare-ghostscript-windows.ps1 -DownloadSource
```

このプロジェクトはGhostscriptを改変していない。将来パッチを加える場合は、
そのパッチとビルド手順を対応するソースコードに含める。

## ライセンス

Ghostscriptおよびpdfx1a-convertはGNU Affero General Public License version 3
の条件に従って配布する。ライセンス全文はポータブルZIPとソースリポジトリの
`LICENSE`に含める。
