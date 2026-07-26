# Open Print PDF

Windows desktop application for PDF inspection and PDF/X-1a conversion.

Status: pre-release.

## Requirements

- Windows 11 x64
- Rust stable
- Node.js 24 or later
- Visual Studio 2022 Build Tools

## Build

```powershell
cargo test --workspace
cd apps/desktop
npm ci
npm run build
```

## Documents

- [License](LICENSE)
- [Third-party notices](THIRD_PARTY_NOTICES.md)
- [Changelog](CHANGELOG.md)
