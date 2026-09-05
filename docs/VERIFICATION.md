# Verification

Latest run: 2026-09-05, Windows 10.0.26200 x64 environment.

## Detected prerequisites

| Item                      | Detected result                                                                         |
| ------------------------- | --------------------------------------------------------------------------------------- |
| Node.js                   | 25.8.1 in the task shell; Corepack subprocess reported 22.23.2 during initial bootstrap |
| Corepack                  | 0.34.6                                                                                  |
| pnpm                      | repository pin 11.22.0; lockfile created                                                |
| Rust                      | 1.91.0 `x86_64-pc-windows-msvc`, installed during Phase 0                               |
| Cargo                     | supplied by Rust 1.91.0 toolchain                                                       |
| Visual Studio Build Tools | PASS — VS 2022 Build Tools 17.14.39, x64 VCTools                                        |
| Windows SDK               | PASS — 10.0.26100.0                                                                     |
| WebView2                  | Evergreen Runtime 152.0.4191.62                                                         |
| Git                       | 2.52.0.windows.1; repository initialized                                                |

## Automated results

| Command                                                 | Result                                                                                       |
| ------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| `pnpm typecheck`                                        | PASS — strict TypeScript project build                                                       |
| `pnpm lint`                                             | PASS — zero warnings                                                                         |
| `pnpm test`                                             | PASS — 2 files, 2 tests                                                                      |
| `pnpm build`                                            | PASS — Vite production build, 1663 modules                                                   |
| `cargo fmt --all --check`                               | PASS                                                                                         |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS — zero warnings                                                                         |
| `cargo test --workspace`                                | PASS — 11 tests across project, persistence, jobs, and credentials                           |
| `pnpm tauri:build`                                      | PASS — optimized `target/release/kiri-desktop.exe`                                           |
| `pnpm test:e2e`                                         | PASS — 6 golden comparisons: Home light/dark/system, naming, source selector, and controller |
| `pnpm phase0:gate`                                      | PASS — complete command succeeded end-to-end on 2026-09-05                                   |

## Native/manual checks

- PASS: launched `target/release/kiri-desktop.exe`; process was responsive with native window title `Kiri`; normal window-close request was accepted.
- PASS (automated domain coverage): project creation writes the required layout and manifest; reopen validates it; autosave persists a mutation for reopen; failed save leaves the previous project readable; schema-zero migration reopens as current.
- PASS (automated): missing recent paths remain indexed and are marked missing.
- PASS (automated/visual): first-paint script resolves theme before React; unit tests cover persistence; six goldens cover system/light/dark, project naming, source selector, and controller; CSS covers reduced motion and visible focus. Light and dark 820×680 goldens were inspected.
- NOT RUN interactively: native folder-dialog create/close/reopen, live Windows system-theme switching, full keyboard traversal, and the 100/125/150/200% DPI matrix. The Windows UI automation helper failed repeatedly with `windows sandbox failed: helper_unknown_error: setup refresh had errors`.

The unavailable click-through does not weaken the passing Rust create/reopen/atomic-save coverage. No unrun manual check is described as passing.
