# ADR-0002: Directory-based portable `.kiri` project

- Status: Accepted
- Date: 2026-09-05

## Context

Creative state and original media must be local, portable, non-destructive, and crash recoverable.

## Decision

A `.kiri` project is a directory with a versioned `project.json` plus media, telemetry, transcript, assets, cache, recovery, and exports directories. Times are integer microseconds or rational frame rates. Asset paths are project-relative. Writes use same-directory temporary files and recovery backups.

## Consequences

Projects are inspectable and portable. Migrations are forward-only and testable. Directory copies are larger than single-file archives but can recover and stream media safely.
