# Implementation status

## Phase 0 — Foundation and Architecture

Status: **complete** (2026-09-05). The complete automated gate and native launch smoke passed; details and unavailable interactive checks are recorded in `docs/VERIFICATION.md`.

- [x] Repository/toolchain audit and Git-safe ignore rules
- [x] pnpm workspace with committed lockfile and pinned Rust toolchain
- [x] Tauri 2 + React + strict TypeScript desktop shell
- [x] Recordly UI audit and independent main/source-selector/controller boundaries
- [x] Semantic system/light/dark theme with first-paint resolution, persistence, live OS updates, reduced motion, and cross-window event synchronization
- [x] Versioned `.kiri` manifest, validation, stable IDs, integer/rational time, relative paths, migration framework, mutation autosave, atomic save/recovery backup, reopen, and Save As
- [x] Migrated SQLite app-data index and missing-project handling
- [x] Cancellable background-job state machine and typed progress payload
- [x] Credential-store interface, memory test implementation, and Windows Credential Manager boundary
- [x] Thin typed Tauri commands and runtime response validation
- [x] Frontend unit tests and browser smoke specifications
- [x] Complete Phase 0 quality gate and native launch smoke

## Deferred by phase

- Phase 1: real source discovery, capture, controller actions, audio/camera/input sources, shortcuts, segmentation, and recovery.
- Phase 2: editor, timeline, debounced editor-to-autosave orchestration, renderer, and MP4 export.
- Phase 3: presentation intelligence, captions, processing, and presets.
- Phase 4: AI providers and Playwright walkthrough execution.
- Phase 5: MCP client/server execution and permission policy.
- Phase 6: installer/signing hardening, advanced export, and 4K performance.

## Next gate

Phase 0 gate passed. The exact next phase is **Phase 1 — Native recording vertical slice**; do not begin it without an explicit request.
