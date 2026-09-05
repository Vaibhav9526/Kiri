# Kiri product context

Kiri is a premium, local-first Windows desktop application for one product builder to record and edit polished product walkthroughs. The Phase 0 surface is an operational, compact launch window—not a SaaS dashboard. Recordly controls placement and interaction density; Kiri's PRD controls product behavior, architecture, security, and semantic color tokens.

## Phase 0 user task

Create a portable empty `.kiri` project, see recent local projects, and reopen an existing project. Capture, AI, and media-import actions are visible only to establish the eventual launch hierarchy and are explicitly unavailable in this development phase.

## Commitments

- Windows 10 build 19041+, x64 first, Tauri 2, React/TypeScript, Rust.
- System/light/dark theme, system by default, no first-paint flash.
- Compact Recordly-shaped windows and controls; no generic navigation dashboard.
- Local-only persistence and no account, cloud, analytics, billing, or invented brand artwork.
- Supplied Kiri assets remain untouched; only valid existing assets may be copied for production use.

## Assumptions

- Personal-use workflow means recent projects prioritize title, path, updated time, and missing state.
- The reference images are valid Kiri brand inputs, but the shell uses only the compact icon asset in Phase 0.
- A native directory chooser is required for project creation and a native folder chooser for opening `.kiri` directories.
