# ADR-0005: Windows capture pipeline

- Status: Accepted
- Date: 2026-09-05

## Context

Phase 1 needs synchronized, recoverable display/window video, microphone, loopback audio, camera, and input telemetry without coupling media services to Tauri.

## Decision

kiri-capture owns Windows Graphics Capture and receives GPU-backed Direct3D surfaces on a free-threaded callback. It submits them directly to Media Foundation H.264. Callbacks never block: bounded queues use drop-newest semantics and report drops. Screen, WASAPI microphone, WASAPI loopback, Media Foundation camera, and the buffered mouse hook use independent workers and share a QueryPerformanceCounter origin.

Pause finalizes the current media segment; resume starts another on the same monotonic clock. Recovery metadata is atomically committed before capture and after finalization. Sources remain independent. Cursor pixels and WGC borders are disabled when supported; the controller is a secondary-window exclusion target.

## Consequences

The path is efficient and Windows-specific. Independently finalized segments survive failures. Multi-segment playback belongs to later editor work. Device hot-plug recovery, source thumbnails, and the startup recovery prompt remain required before this phase gate can pass.
