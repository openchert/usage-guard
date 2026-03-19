# UsageGuard Project

> Historical note: parts of this document predate the security hardening pass.
> Current secret-storage and local consumer behavior is documented in `docs/SECURITY.md` and `docs/LOCAL_CONSUMER_SOURCES.md`.
> Current provider/source display behavior is documented in `docs/PROVIDERS.md`.

UsageGuard is a local-first AI usage monitor built as a Rust workspace. It normalizes usage data from multiple providers into one shared snapshot model, then exposes that data through a CLI and a compact desktop widget.

## Workspace layout

- `crates/usageguard-core`: config loading, OS-native secret storage, provider adapters, snapshot normalization, alert evaluation, and demo fallback data.
- `crates/usageguard-cli`: terminal entrypoint for demo output, local alert checks, and basic config helpers.
- `crates/usageguard-desktop`: Tauri desktop runtime, tray integration, native notifications, widget window management, and menu handling.
- `crates/usageguard-desktop/ui`: Svelte 5 + Vite frontend for the mini desktop widget.
- `examples/providers`: sample provider profile and NDJSON log inputs.
- `docs`: project and interface documentation.

## Snapshot pipeline

`usageguard-core` uses two different acquisition paths depending on the provider type.

### Local consumer sources

- OpenAI / Codex:
  fresh `~/.codex/sessions/**/*.jsonl` quota data first, then a built-in fallback fetch using the local token from `~/.codex/auth.json`, then a local status snapshot if Codex is signed in but quota data is not available yet.
- Anthropic / Claude Code:
  local plan metadata from `~/.claude/.credentials.json`, then a built-in usage fetch using the local token from that file, then a local status snapshot if Claude Code is signed in but quota data is not available yet.

### API and admin monitoring sources

For API/admin providers, `usageguard-core` resolves data in this order:

1. Provider usage log from `*_USAGE_LOG` if present.
2. HTTP API call using configured endpoint and API key.
3. Environment fallback such as `*_SPENT_USD` and `*_LIMIT_USD`.
4. Demo data if no provider returns anything.

Every provider is normalized into the shared `UsageSnapshot` model with:

- provider id
- account label
- spend and limit in USD
- input and output token counts
- inactivity in hours
- source label
- optional safe status code/message fields

Alerts are evaluated after snapshot collection. Current alert logic covers:

- local consumer quota alerts for `5h` and `week` windows where available
- near-limit warnings when quota is almost exhausted
- use-before-reset reminders when reset is close and usage is still low
- legacy budget and inactivity alerts for API/admin monitoring sources

Quiet hours suppress non-critical notifications. Active alerts also surface in the widget card state, while native notifications are emitted when a new alert signature becomes active.

## Current desktop behavior

The current desktop app is a Tauri 2 widget, not the older `eframe/egui` shell.

- Frameless, transparent, compact widget window.
- Starts in the bottom-right corner of the active monitor.
- Uses the Windows work area on startup so the widget stays above the taskbar.
- Resizes horizontally to fit the number of provider cards.
- Refreshes snapshots every 30 seconds.
- Left mouse drag moves the widget.
- A small add button opens provider management.
- Right-click opens the platform native context menu.
- Tray left-click toggles show/hide.
- Tray and context menus expose provider management.
- A dedicated native settings window manages provider accounts.
- Native notifications are emitted on Linux, Windows, and macOS when alert signatures change.
- Widget cards keep a visible alert badge and border tint while an alert remains active.

## Configuration and secrets

- Shared config is stored at the OS config directory under `usage-guard/config.json`.
- On Windows, provider API keys are stored in `%APPDATA%\usage-guard\secrets.bin` using DPAPI.
- On Linux, provider API keys are stored in the desktop Secret Service keyring.
- Legacy browser-sign-in storage from older builds is cleaned up on load and is not used by the current runtime.
- Named provider accounts are stored in `provider_accounts` and can reuse the same vendor multiple times with different labels and keys.
- The first hardened desktop deploy is locked to built-in audited endpoints for outbound fetches.
- Legacy custom provider profiles are no longer used for outbound fetches.

## Development workflow

Prerequisites:

- Rust toolchain
- Node.js and npm for the desktop UI

Common commands from the repository root:

```bash
npm install --prefix crates/usageguard-desktop/ui
cargo test
cargo run -p usageguard-cli -- demo
cargo run -p usageguard-desktop
```

The desktop build uses Tauri's `beforeBuildCommand` to build the Svelte UI from `crates/usageguard-desktop/ui`.

## Related docs

- `docs/LOCAL_CONSUMER_SOURCES.md`: exact local-file and token-backed consumer acquisition flow.
- `docs/SECURITY.md`: secret storage, local credential usage boundaries, and threat model.
- `docs/PROVIDERS.md`: provider/source display adapters and normalized card behavior.
- `docs/SESSION_2026-03-08_DESKTOP_REWRITE.md`: recap of the desktop rewrite and native context menu work.
- `docs/ALERTS.md`: quota alert thresholds, reminder windows, and delivery behavior.
