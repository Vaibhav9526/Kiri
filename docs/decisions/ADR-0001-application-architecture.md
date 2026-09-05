# ADR-0001: Tauri host with independent Rust domains

- Status: Accepted
- Date: 2026-09-05

## Context

Kiri needs Windows-native media performance without coupling project/media logic to its WebView or desktop lifecycle.

## Decision

Use a Tauri 2 host, React/TypeScript UI, and Rust workspace. Tauri commands remain adapters. Phase 0 creates only project, persistence, jobs, and credentials crates.

## Consequences

Domain crates can be tested without the UI. Native dependencies and IPC contracts remain explicit. More plumbing is required than placing business logic in handlers.
