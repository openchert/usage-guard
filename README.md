# UsageGuard

**Know your AI usage in 5 seconds, without getting distracted.**

UsageGuard is a calm, local-first AI usage monitor with:
- **Desktop mini app** (frameless, low-distraction)
- **CLI** for terminal workflows
- Cross-platform target: Windows / macOS / Linux

> Minimal UI. Meaningful alerts. Local control.

## Docs
- [`docs/SECURITY.md`](docs/SECURITY.md): secure storage, OAuth flow, migration, and Tauri hardening
- [`docs/PROVIDERS.md`](docs/PROVIDERS.md): provider/source modules, snapshot schema, and built-in fetch policy

## Suggested GitHub metadata
**About blurb:**
> Calm, local-first AI usage monitor with a minimal desktop UI + CLI. Track spend, quotas, and limits across multiple providers without dashboard noise.

**Topics:**
`ai` `usage-tracking` `quota-tracker` `cost-monitoring` `developer-tools` `rust` `tauri` `svelte` `desktop-app` `cli` `open-source` `local-first` `openai` `anthropic`

## Stack
- Rust workspace
- Desktop: `Tauri 2` + `Svelte 5` + `Vite`
- CLI: `clap`
- Shared core: alerts, snapshots, quiet-hours logic

## Run

```bash
npm install --prefix crates/usageguard-desktop/ui
cargo test
cargo run -p usageguard-cli -- demo
cargo run -p usageguard-desktop
```

The desktop crate builds the UI through Tauri's `beforeBuildCommand`, so Node.js and npm are required for local desktop runs.

## Installation (end users - no Rust required)
Download the latest release from GitHub Releases and use the package for your OS.

### One-command install
(Installs latest GitHub Release binaries)

#### Windows (PowerShell)
```powershell
irm https://raw.githubusercontent.com/openchert/usage-guard/main/install.ps1 | iex
```

#### macOS / Linux (bash)
```bash
curl -fsSL https://raw.githubusercontent.com/openchert/usage-guard/main/install.sh | bash
```

### Windows (x64)
1. Download: `usage-guard-windows-x64.zip`
2. Extract the zip.
3. Run:
   - `usageguard-desktop.exe` (desktop app)
   - `usageguard.exe` (CLI)

PowerShell example:
```powershell
./usageguard.exe demo
./usageguard.exe config --show
```

### macOS (Apple Silicon / arm64)
1. Download: `usage-guard-macos-arm64.tar.gz`
2. Extract:
```bash
tar -xzf usage-guard-macos-arm64.tar.gz
```
3. Run:
```bash
chmod +x usageguard usageguard-desktop
./usageguard-desktop
```
CLI example:
```bash
./usageguard demo
./usageguard config --show
```

### Linux (x64)
1. Download: `usage-guard-linux-x64.tar.gz`
2. Extract:
```bash
tar -xzf usage-guard-linux-x64.tar.gz
```
3. Run:
```bash
chmod +x usageguard usageguard-desktop
./usageguard-desktop
```
CLI example:
```bash
./usageguard demo
./usageguard config --show
```

## Verify release integrity (recommended)
Each release includes `SHA256SUMS`.

### macOS/Linux
```bash
sha256sum -c SHA256SUMS
```

### Windows (PowerShell)
```powershell
Get-FileHash .\usage-guard-windows-x64.zip -Algorithm SHA256
```
Compare with the hash in `SHA256SUMS`.

## API setup quickstart
Desktop:
1. Open `usageguard-desktop`
2. Click the `+` button on the widget or open the native right-click menu
3. Choose **Manage Providers...**
4. Select a vendor
5. Enter a display name and API key
6. Save the provider

The desktop settings window supports multiple named accounts per vendor.

On Windows, API keys and OAuth refresh tokens are stored in a DPAPI-encrypted blob at `%APPDATA%\usage-guard\secrets.bin`. OpenAI OAuth access tokens stay in memory only and are refreshed when needed.

For the first hardened deploy, provider fetches are locked to built-in audited endpoints only. Custom endpoint overrides and custom provider profiles are not used for outbound requests.

OpenAI ChatGPT OAuth uses the system browser plus a localhost callback at `http://localhost:1455/auth/callback`. The app validates the callback path and OAuth `state` before exchanging the code.

CLI:
```bash
usageguard config --openai-key "sk-..."
usageguard config --anthropic-key "sk-ant-..."
usageguard demo
```

## Provider support
Current first-deploy remote sources:
- OpenAI OAuth via `https://chatgpt.com/backend-api/wham/usage`
- OpenAI API via the built-in organization costs endpoint
- Anthropic API via the built-in organizations usage endpoint
- Cursor API via the built-in team spend endpoint

The UI display path is modular per provider and source, so OAuth/API variants can interpret their own raw usage semantics without hardcoding that logic in the main widget.

See [`docs/SECURITY.md`](docs/SECURITY.md) for storage and OAuth details.
See [`docs/PROVIDERS.md`](docs/PROVIDERS.md) for provider/source display modules and snapshot semantics.

## Troubleshooting
- If install command succeeds but command not found, restart terminal (PATH refresh).
- If ChatGPT OAuth sign-in fails, make sure nothing else is using `localhost:1455`.
- If the widget shows `Status: Unable to load provider usage right now.`, verify the API key and that the provider account has access to the supported endpoint.
- If secure storage is unavailable, the app does not fall back to plaintext secret persistence.
- If no API/log/env source is available, app falls back to demo data by design.

## Release build automation
- GitHub Actions builds binaries on tag push (`v*`) for:
  - Linux (x64)
  - Windows (x64)
  - macOS (arm64)
- Artifacts are attached to GitHub Releases as `.tar.gz`/`.zip`.

Create a release build:
```bash
git tag v0.4.1
git push origin v0.4.1
```

## Current UI behavior
- Frameless transparent widget window
- Bottom-right startup placement with Windows taskbar-aware positioning
- Draggable with the left mouse button
- Small add button on the widget for provider setup
- Native right-click menu with `Refresh`, `Manage Providers...`, `Always on Top`, `Hide to Tray`, and `Quit`
- Tray left-click toggles visibility
- Tray and native settings window for provider management
- Refreshes snapshot data every 30 seconds
- Native desktop notifications on Linux, Windows, and macOS when alert signatures change
- Multiple named accounts per vendor are displayed directly in the widget
- Provider cards show compact `5h` and `week` usage rings

## CLI config helpers
```bash
cargo run -p usageguard-cli -- config --show
cargo run -p usageguard-cli -- config --openai-key "sk-..."
cargo run -p usageguard-cli -- config --anthropic-key "sk-ant-..."
```

## Note
UsageGuard is intentionally minimal and non-distracting.
Current roadmap focus: provider parity hardening, native notification parity (Windows/macOS), and signed/notarized releases.
