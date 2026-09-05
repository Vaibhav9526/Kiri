# Security

## Phase 0 trust boundaries

- The WebView invokes only named Tauri commands with structured request objects.
- No arbitrary executable strings cross IPC.
- Project manifests are untrusted input: Rust deserializes and validates schema version, IDs, time values, and relative paths.
- In-project paths cannot be absolute, rooted, prefixed, or contain parent traversal.
- Provider secrets never enter frontend storage, project manifests, logs, or SQLite. `kiri-credentials` exposes an interface whose Windows implementation uses the operating-system credential store.
- SQLite contains metadata placeholders only, never credential material.
- Network access is not required for normal Phase 0 runtime behavior.
- Runtime logs and crash reports stay under local app data and are never uploaded. Future sensitive values must be redacted before logging.

## Save safety

Manifests are written to a temporary file in the project directory, flushed, and persisted at the target boundary. Before replacing an existing manifest, Kiri copies the prior readable version to `recovery/project.json.bak`. A validation/write failure leaves the prior manifest readable.

## Deferred policy

Provider consent, MCP Ask/Deny policy, capture exclusion, browser-profile isolation, log redaction, and destructive-action approvals are specified by the PRD and implemented only in their scheduled phases.
