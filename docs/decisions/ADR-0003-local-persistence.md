# ADR-0003: Migrated SQLite machine index

- Status: Accepted
- Date: 2026-09-05

## Context

Recent projects and machine preferences must not pollute portable creative state.

## Decision

Store local metadata in SQLite under the Tauri app-data directory. Every schema change is an ordered, recorded migration. Credentials are excluded.

## Consequences

Recent paths may become missing and are reported rather than deleted. The database can be rebuilt without losing project contents.
