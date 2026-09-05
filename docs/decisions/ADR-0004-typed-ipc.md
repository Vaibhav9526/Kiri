# ADR-0004: Named typed IPC and progress events

- Status: Accepted
- Date: 2026-09-05

## Context

The WebView is a separate trust boundary and future jobs will be cancellable and long-running.

## Decision

Use named Tauri commands with structured Rust request/response types. Validate responses in TypeScript with Zod. Use serializable job progress events with explicit state, counts, message, and timestamp. Never accept executable command strings.

## Consequences

Boundary mismatches fail visibly. Some types are represented in both languages in Phase 0; generated schema automation can replace this small bootstrap without changing the contract.
