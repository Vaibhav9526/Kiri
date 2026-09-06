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

## Phase 1 latest results — 2026-09-05

- Phase 0 gate rerun before implementation: PASS.
- cargo clippy --workspace --all-targets -- -D warnings: PASS.
- cargo test --workspace: PASS (20 tests).
- pnpm lint: PASS.
- pnpm typecheck: PASS.
- pnpm test: PASS (2 tests).
- pnpm test:e2e: PASS after intentional golden update (6 tests).
- WGC H.264 hardware spike: PASS — 1920x1080 at declared 60 fps, 136 captured frames in 5.052 s; ffprobe reports H.264 video, 4.983317 s duration, GPU path. The windows-capture encoder also emits a silent AAC stream when audio is disabled; product screen segments must account for this during muxing.
- Complete automated quality command (with Cargo on PATH): PASS, including optimized Tauri build at target/release/kiri-desktop.exe.

30-minute protocol: record visible/audible clap markers at start/end with all sources; retain diagnostics; run ffprobe -v error -show_streams -show_format -of json on each segment; compare marker offsets and document drift, drops, queue depth and device events.

## Phase 1 completion results — 2026-09-06

- `pnpm phase1:gate`: PASS through formatting, lint, strict typecheck, frontend unit/golden tests, Rust formatting/clippy/tests, and production frontend compilation. The first release-link attempt was blocked by a running test executable; after stopping only workspace-built Kiri test processes, `pnpm tauri:build` passed and produced `target/release/kiri-desktop.exe`.
- Display capture: H.264 1920x1080/60 metadata, 29.966650 s, 1,025 delivered frames, zero queue drops. Available hardware/change-driven WGC delivery averaged 34.1 callbacks/s on the 30-second desktop sample; the limitation is measured, not hidden.
- Window capture: Chrome H.264 1550x830/60, 8.033317 s, 333 frames, zero drops.
- Combined 10-second run: screen 9.999983 s; microphone PCM float stereo 48 kHz 9.70 s; loopback PCM float stereo 48 kHz 9.83 s; camera H.264 1280x720/30 9.80 s. Per-source ready offsets are committed against the QPC recording clock.
- Longest practical audio drift sample: 60 seconds; microphone 59.93 s and loopback 60.08 s. The 150 ms file-length difference includes measured independent startup/stop boundaries and is aligned by stored ready offsets. The documented 30-minute marker protocol remains repeatable.
- Camera enumeration returned 13 formats through Media Foundation, including 1280x720/30 and 1920x1080/30. Camera recording produced H.264 1280x720/30.
- WGC thumbnail: PASS; display JPEG data URL was 310,887 bytes.
- Pause/resume segmentation: two independently finalized H.264 1920x1080/60 segments, each 3.016650 s and ffprobe-readable.
- Forced termination: PASS; process killed while segment two was active, recovery replay found one finalized playable 1920x1080/30 three-second segment and ignored the unfinished segment.
- Source closure: dedicated window terminated during capture; finalized H.264 media remained ffprobe-readable (2.566633 s).
- DPI: automated physical/logical mapping passes for 100%, 125%, and 150%; the available desktop was observed at 125%. Changing Windows system DPI was not performed.
- Native GUI automation: unavailable because the Windows control kernel exited twice during initialization. Playwright goldens cover Home and capture surfaces in light/dark/system; native capture diagnostics exercise the Rust services without React.
