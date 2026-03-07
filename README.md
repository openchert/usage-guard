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
