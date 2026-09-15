# Attribution notice

Kiri's recording and editor architecture is derived from
[Recordly](https://github.com/webadderallorg/Recordly) by webadderall,
which is licensed under the GNU Affero General Public License v3.0.

- Upstream: https://github.com/webadderallorg/Recordly
- Kiri keeps its own name and branding; "Recordly" branding is not used.
- The Electron main process, native helpers, and React editor structure
  from Recordly were ported to a Tauri 2 + Rust backend with a React
  frontend. AI features (local captions/transcription and later AI
  walkthrough work) are deferred behind explicit phases.
- As a derivative of an AGPLv3 work, this repository is licensed under
  `AGPL-3.0-or-later`. See `Cargo.toml` (`workspace.package.license`).
  The full license text follows the upstream project until a vendored
  copy is added before release.
