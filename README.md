# usage-guard (Rust Edition)

A small, local-first AI usage monitor with:
- **Desktop mini app** (frameless, low-distraction)
- **CLI** for terminal workflows
- Cross-platform target: Windows / macOS / Linux

## Stack
- Rust workspace
- Desktop: `eframe/egui` (frameless mini window)
- CLI: `clap`
- Shared core: alerts, snapshots, quiet-hours logic

## Run

```bash
source $HOME/.cargo/env
cargo test
cargo run -p usageguard-cli -- demo
cargo run -p usageguard-desktop
```

## Installation (end users — no Rust required)
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
2. Click **Connect API**
3. Paste API key(s)
4. (Optional) set custom endpoint URL(s)
5. Save and click **Refresh**

CLI:
```bash
usageguard config --openai-key "sk-..."
usageguard config --anthropic-key "sk-ant-..."
usageguard config --openai-endpoint "https://api.openai.com/v1/organization/costs"
usageguard config --anthropic-endpoint "https://api.anthropic.com/v1/organizations/usage"
usageguard demo
```

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
- No top bar/window decorations
- Compact information-focused panel
- Usage visualization + alert text
- "Connect API" opens a tiny local key form and saves to local config

## CLI config helpers
```bash
cargo run -p usageguard-cli -- config --show
cargo run -p usageguard-cli -- config --openai-key "sk-..."
cargo run -p usageguard-cli -- config --anthropic-key "sk-ant-..."
```

## Note
This milestone keeps the app intentionally minimal and non-distracting.
Next step is replacing placeholder connected-state snapshots with full provider usage API adapters.
