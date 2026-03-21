# UsageGuard Project

> Historical note: parts of this document predate the consumer-only cleanup.
> Current local consumer behavior is documented in `docs/SECURITY.md` and `docs/LOCAL_CONSUMER_SOURCES.md`.
> Current provider/source display behavior is documented in `docs/PROVIDERS.md`.

UsageGuard is a local-first AI usage monitor built as a Rust workspace. It normalizes local Codex and Claude Code usage into one shared snapshot model, then exposes that data through a CLI and a compact desktop widget.

## Workspace layout

- `crates/usageguard-core`: config loading, local consumer adapters, snapshot normalization, and alert evaluation.
- `crates/usageguard-cli`: terminal entrypoint for local status inspection.
- `crates/usageguard-desktop`: Tauri desktop runtime, tray integration, native notifications, widget window management, and menu handling.
- `crates/usageguard-desktop/ui`: Svelte 5 + Vite frontend for the mini desktop widget and connections window.
- `docs`: project and interface documentation.

## Snapshot pipeline

`usageguard-core` now uses only local consumer acquisition paths.

### Local consumer sources

- OpenAI / Codex:
  fresh `~/.codex/sessions/**/*.jsonl` quota data first, then a built-in fallback fetch using the local token from `~/.codex/auth.json`, then a local status snapshot if Codex is signed in but quota data is not available yet.
- Anthropic / Claude Code:
  local plan metadata from `~/.claude/.credentials.json`, then a built-in usage fetch using the local token from that file, then a local status snapshot if Claude Code is signed in but quota data is not available yet.

Every provider is normalized into the shared `UsageSnapshot` model with:

- provider id
- account label
- source label
- optional safe status code and message fields
- normalized consumer quota windows when available
- optional reset timestamps for `5h` and `week`

Alerts are evaluated after snapshot collection. Current alert logic covers:

- local consumer quota alerts for `5h` and `week` windows where available
- near-limit warnings when quota is almost exhausted
- use-before-reset reminders when reset is close and usage is still low

Quiet hours suppress non-critical notifications. Active alerts also surface in the widget card state, while native notifications are emitted when a new alert signature becomes active.

## Current desktop behavior

The current desktop app is a Tauri 2 widget.

- Frameless, transparent, compact widget window.
- Starts in the bottom-right corner of the active monitor.
- Uses the work area on startup so the widget stays on-screen.
- Resizes horizontally to fit the number of connection cards.
- Refreshes snapshots on a configurable interval.
- Left mouse drag moves the widget.
- Right-click opens the platform native context menu.
- Tray left-click toggles show and hide.
- Tray and context menus expose `Manage Connections...`.
- A dedicated native connections window manages local labels and alert toggles.
- Native notifications are emitted when alert signatures change.
- Widget cards keep a visible alert badge and border tint while an alert remains active.

## Configuration

- Shared config is stored at the OS config directory under `usage-guard/config.json`.
- Current persisted settings are limited to quiet hours, refresh interval, theme, widget position, local connection labels, and per-window alert toggles.
- Legacy remote-monitoring fields from older builds are ignored during load and removed on the next save.

## Development workflow

Prerequisites:

- Rust toolchain
- Node.js and npm for the desktop UI

Common commands from the repository root:

```bash
npm install --prefix crates/usageguard-desktop/ui
cargo test
cargo run -p usageguard-cli -- status
cargo run -p usageguard-desktop
```

The desktop build uses Tauri's `beforeBuildCommand` to build the Svelte UI from `crates/usageguard-desktop/ui`.

## Related docs

- `docs/LOCAL_CONSUMER_SOURCES.md`: exact local-file and token-backed consumer acquisition flow.
- `docs/SECURITY.md`: local credential usage boundaries and threat model.
- `docs/PROVIDERS.md`: provider and display model.
- `docs/SESSION_2026-03-08_DESKTOP_REWRITE.md`: recap of the desktop rewrite and native context menu work.
- `docs/ALERTS.md`: quota alert thresholds, reminder windows, and delivery behavior.
