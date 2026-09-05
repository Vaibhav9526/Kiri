# Kiri architecture

Phase 0 establishes a Tauri 2 host around a React/TypeScript launch UI and focused Rust domain crates. The frontend owns transient presentation state; portable creative state belongs to `kiri-project`; machine-local index/settings metadata belongs to `kiri-persistence`; background lifecycle semantics belong to `kiri-jobs`; secrets are available only behind `kiri-credentials`.

## Boundaries

- `apps/desktop`: Vite/React frontend plus thin Tauri commands and window configuration.
- `crates/kiri-project`: versioned manifest, validation, migration, portable directory creation, atomic save, reopen, and Save As.
- `crates/kiri-persistence`: ordered SQLite migrations and machine-local metadata access.
- `crates/kiri-jobs`: cancellable job state machine and serializable progress messages.
- `crates/kiri-credentials`: provider-independent secret interface, memory test store, and Windows Credential Manager adapter.

Structured Tauri logs and plain-text panic reports are written locally through the desktop host. Panic reports live in the app-data `crashes` directory and never upload automatically.

The project crate has no Tauri dependency. Tauri handlers translate typed requests into domain calls and return typed summaries. SQLite is created under Tauri's application-data directory. Project media paths are normalized relative paths and may not escape the `.kiri` root.

## Phase 0 data flow

```text
Home -> typed invoke -> thin Tauri command -> kiri-project -> .kiri/project.json
                                      |----> kiri-persistence -> app-data/kiri.db
Theme selector -> root data attributes + local mode -> Tauri event -> other windows
```

## State ownership

- Persistent project state: `project.json`, loaded and validated by Rust.
- Persistent machine state: SQLite recent projects/settings/preset/job/provider/MCP metadata.
- Transient UI state: component state such as active notice and busy state.
- Background job state: canonical Rust `Job`; frontend receives typed `JobProgress` events.

Capture, render, export, AI, automation, and MCP execution are intentionally absent until their phases.

## Phase 1 capture data flow

The Tauri layer validates typed requests and coordinates services. kiri-capture owns WGC, the QPC clock, and recovery metadata; kiri-audio owns separate shared-mode WASAPI microphone and loopback workers; kiri-camera owns Media Foundation camera capture; kiri-input owns cursor/click JSONL. Each source writes separate project segments and the recovery manifest is the incremental commit boundary.
