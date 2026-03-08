# UsageGuard

**Know your AI usage in 5 seconds, without getting distracted.**

UsageGuard is a calm, local-first AI usage monitor with:
- **Desktop mini app** (frameless, low-distraction)
- **CLI** for terminal workflows
- Cross-platform target: Windows / macOS / Linux

> Minimal UI. Meaningful alerts. Local control.

## Docs
- `docs/PROJECT.md`: current project overview, architecture, and development workflow
- `docs/SESSION_2026-03-08_DESKTOP_REWRITE.md`: recap of the Tauri desktop rewrite and native context menu work
- `docs/INTERFACES.md`: provider adapter contracts and normalized snapshot schema
- `docs/ADAPTER_EXAMPLES.md`: custom provider and NDJSON examples
- `docs/NEXT_STEPS.md`: public roadmap summary

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
6. Add an endpoint URL when the selected provider requires one
7. Save the provider

The desktop settings window supports multiple named accounts per vendor, so you can add more than one OpenAI, Anthropic, or other supported account.

API keys are stored in the OS keyring when available instead of plain config JSON.

Copilot requires a GitHub organization premium-request usage endpoint URL. Cursor uses the built-in `https://api.cursor.com/teams/spend` endpoint by default.

CLI:
```bash
usageguard config --openai-key "sk-..."
usageguard config --anthropic-key "sk-ant-..."
usageguard config --openai-endpoint "https://api.openai.com/v1/organization/costs"
usageguard config --anthropic-endpoint "https://api.anthropic.com/v1/organizations/usage"
usageguard demo
```

## Provider support
UsageGuard now includes built-in adapters for:
- OpenAI
- Anthropic
- Gemini
- Mistral
- Groq
- Copilot
- Cursor

Plus custom provider profiles via config.

See `docs/PROJECT.md` for the current workspace layout and runtime behavior.
See `docs/INTERFACES.md` for exact environment variables, headers, endpoint contracts, and normalized schema.
See `docs/ADAPTER_EXAMPLES.md` for custom provider/profile examples.
See `docs/NEXT_STEPS.md` for the public short roadmap.

## Troubleshooting
- If install command succeeds but command not found, restart terminal (PATH refresh).
- If API shows `source: api-error:...`, verify key permissions and endpoint URL.
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
