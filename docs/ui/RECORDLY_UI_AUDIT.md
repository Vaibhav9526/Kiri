# Recordly UI audit for Kiri

Audit target: current `webadderallorg/Recordly` main branch and v1.3.3 release available on 2026-09-05. Recordly is AGPL-3.0; Phase 0 uses a clean-room structural adaptation based on public source inspection and the Kiri placement contract. No Recordly source, name, logo, artwork, updater, telemetry, Electron code, or marketplace code is copied.

| Recordly source                       | Responsibility                  | Kiri counterpart                                    | Action  | Reason                                                                                                         |
| ------------------------------------- | ------------------------------- | --------------------------------------------------- | ------- | -------------------------------------------------------------------------------------------------------------- |
| `src/App.tsx`                         | Route by window type            | `apps/desktop/src/App.tsx`                          | adapt   | Retain separate launch/source/controller surfaces using Tauri labels instead of query-driven Electron windows. |
| `ThemeContext.tsx`                    | Persist and resolve theme       | `theme/ThemeProvider.tsx`, first-paint script       | port    | Preserve system/light/dark behavior; synchronize with Tauri events and Kiri semantic tokens.                   |
| `src/index.css`, `launchTheme.css`    | Global and launch appearance    | `styles/tokens.css`, `styles/index.css`             | adapt   | Preserve compact density and state hierarchy with Kiri's locked palette.                                       |
| `LaunchWindow.tsx`                    | Compact launch composition      | `components/Home.tsx`                               | adapt   | Keep a focused creation surface and recent projects without a dashboard rail.                                  |
| `RecordingControls.tsx`               | Floating recording HUD          | `components/RecordingController.tsx`                | adapt   | Establish a separate 280×48 always-on-top boundary; controls are intentionally disabled until Phase 1.         |
| `SourceSelector.tsx`                  | Focused screen/window selection | `components/SourceSelector.tsx`                     | adapt   | Establish a frameless 620×420 always-on-top boundary without fake sources.                                     |
| launch popovers                       | Anchored compact controls       | theme segmented control and inline status           | adapt   | Phase 0 needs only appearance/status; later capture popovers are deferred.                                     |
| editor layout components              | Editor-region placement         | none in Phase 0                                     | exclude | Editor implementation begins in Phase 2; the placement contract remains binding.                               |
| timeline components                   | Multi-track timeline            | none in Phase 0                                     | exclude | No timeline behavior is permitted in Phase 0.                                                                  |
| editor settings panels                | Contextual inspector            | none in Phase 0                                     | exclude | No editor settings exist yet.                                                                                  |
| `components/ui` primitives            | Compact reusable controls       | local launch controls/theme selector                | adapt   | Port only primitives required now; avoid importing unused surface area.                                        |
| project/timeline/appearance/UI stores | State ownership split           | Rust project/index + ThemeProvider + local UI state | adapt   | Canonical project state stays in Rust; transient presentation state stays in React.                            |
| Electron preload/windows/filesystem   | Native bridge                   | Tauri commands/events/windows                       | exclude | Kiri has one Tauri backend and no Electron runtime.                                                            |

## Reference capture status

The Recordly application was not cloned or executed in this clean workspace because its development toolchain and media fixtures are not project dependencies, and direct source reuse would introduce AGPL obligations. Public source and current repository metadata were inspected instead. Before a later UI-fidelity phase, capture the following against the then-current Recordly revision and retain the revision SHA beside the images:

1. Clone Recordly to a directory outside Kiri and follow its own documented setup.
2. Run its desktop development build at 100% Windows scaling.
3. Capture PNGs at native window dimensions for: launch/controller, source selector, editor dark, editor light, settings inspector, preset menu, export menu, and a populated multi-track timeline.
4. Repeat launch/source/editor at 125%, 150%, and 200% scaling; record OS theme and viewport dimensions in filenames.
5. Store captures under `docs/ui/reference/recordly/<revision>/` only if licensing review permits committing them. Otherwise record local absolute paths in the verification log without committing third-party artwork.

## Kiri comparison viewport

The Phase 0 baseline is the main window at 820×680 and source selector at 620×420, matching Recordly's compact multi-window proportions. Automated screenshots cover light, dark, and system modes at 820×680. Phase 0 compares placement and theme-critical states, not editor regions that do not yet exist.
