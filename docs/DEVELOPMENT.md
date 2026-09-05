# Windows developer setup

## Prerequisites

- Windows 10 build 19041 or later, x64.
- Node.js 22 LTS or newer and Corepack.
- pnpm 11.22.0 (selected through the committed `packageManager` field).
- Rust 1.91.0 with `x86_64-pc-windows-msvc`, rustfmt, and clippy (selected by `rust-toolchain.toml`).
- Visual Studio 2022 Build Tools with **Desktop development with C++**, MSVC x64 tools, and a Windows 10/11 SDK.
- Microsoft Edge WebView2 Evergreen Runtime. Windows 11 includes it; current Windows 10 installations normally do as well.

## Setup and development

```powershell
corepack enable
pnpm install --frozen-lockfile
pnpm tauri:dev
```

## Quality commands

```powershell
pnpm format:check
pnpm lint
pnpm typecheck
pnpm test
pnpm test:e2e
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm tauri:build
pnpm phase0:gate
```

The final command is the complete Phase 0 automated gate. Run it from a Developer PowerShell for VS 2022 if `link.exe` is not available in an ordinary shell.
