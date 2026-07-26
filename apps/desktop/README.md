# Open Print PDF Desktop

Tauri 2、Vite、TypeScriptで構成したOpen Print PDFのデスクトップUIです。

```bash
npm install
npm run tauri dev
```

PDF検査はGhostscriptなしで動作します。アウトライン化とPDF/X-1a変換にはGhostscriptが必要です。

フロントエンドから直接PDFを書き換えず、Tauri command経由で`open-print-pdf-application`を呼び出します。透過を含むPDF/X-1a変換では、UIの確認に加えてRust側でも明示的な許可を検証します。
