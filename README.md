<p align="center">
  <img src="public/assets/title.png" alt="UsageGuard" width="480">
</p>

<h1 align="center">UsageGuard</h1>
<p align="center">A local-first Windows widget and CLI for tracking AI spend, quotas, and subscription usage without dashboard noise.</p>
<p align="center"><strong>Windows x64 release | Linux/macOS source-only for now | No Rust required on Windows</strong></p>

UsageGuard keeps provider usage visible in a small desktop widget instead of burying it across multiple dashboards. It runs locally on Windows and stores your credentials securely on your machine.

## What It Does
- Tracks ChatGPT and Claude subscription quotas plus OpenAI and Anthropic org/admin API usage in one widget
- Shows compact cards with hover details for usage, spend, tokens, requests, reset times, and status
- Sends native desktop notifications and shows in-widget alert badges for quota, budget, and inactivity issues
- Supports browser sign-in for ChatGPT and Claude, plus multiple OpenAI and Anthropic monitoring accounts
- Includes widget controls for `Light Mode`, `Always on Top`, `Hide to Tray`, `Refresh`, and tray show/hide
- Stores API keys and refresh tokens securely on Windows and includes an optional CLI

## Install
### Windows
The installer downloads the latest Windows release from GitHub, extracts the binaries, and adds them to your user `PATH`.

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/openchert/usage-guard/main/install.ps1 | iex
```

Windows CMD:

```powershell
curl -L https://raw.githubusercontent.com/openchert/usage-guard/main/install.ps1 -o install-usageguard.ps1
powershell -ExecutionPolicy Bypass -File .\install-usageguard.ps1
```

Manual install:

1. Download `usage-guard-windows-x64.zip` from GitHub Releases.
2. Extract the archive.
3. Run `usageguard-desktop.exe` for the widget or `usageguard.exe` for the CLI.

## Supported Connections
- ChatGPT subscription usage through browser sign-in
- Claude subscription usage through browser sign-in
- OpenAI organization usage through an organization or admin monitoring key
- Anthropic organization usage through an admin monitoring key

API-key monitoring accepts organization/admin keys only. Individual API keys are not supported.

## Alerts
UsageGuard ships with native desktop notifications and an in-widget alert state for the most important quota conditions.

- OAuth subscription sources watch both the `5h` and `week` windows
- Near-limit alerts fire at `90%` used for `5h` and `80%` used for `week`
- Use-before-reset reminders fire when a reset is close and usage is still low
- API/admin monitoring sources keep spend and inactivity alerts

See [`docs/ALERTS.md`](docs/ALERTS.md) for the full alert model and delivery behavior.

## Quick Start
### Desktop widget
1. Launch `usageguard-desktop`.
2. Open **Manage Providers...** from the `+` button, the widget right-click menu, or the tray menu.
3. Connect ChatGPT or Claude, or add an OpenAI or Anthropic monitoring account with an API key.
4. Hover any provider card for details and keep the widget running for notifications and alert badges.

### Optional CLI
```bash
usageguard config --openai-key "sk-..."
usageguard config --anthropic-key "sk-ant-admin-..."
usageguard demo
```

## Updates
- On Windows, update by running the same install command or script again. It always pulls the latest GitHub release and replaces the installed binaries.
- The desktop app now checks GitHub Releases in the background on startup and shows a native notification when a newer version is available.

## Security
On Windows, API keys and OAuth refresh tokens are stored in a DPAPI-encrypted file at `%APPDATA%\usage-guard\secrets.bin`. Access tokens stay in memory only and are refreshed when needed.

UsageGuard does not fall back to plaintext secret storage if secure storage is unavailable.

See [`docs/SECURITY.md`](docs/SECURITY.md) for storage, OAuth, and threat-model details.
See [`docs/ALERTS.md`](docs/ALERTS.md) for alert thresholds, native notifications, and widget badges.
See [`docs/PROVIDERS.md`](docs/PROVIDERS.md) for the provider/source display model.

## Troubleshooting
- If the install command succeeds but `usageguard` is not found, restart the terminal so `PATH` is reloaded.
- If `irm` is unavailable, use `Invoke-RestMethod`, `Invoke-WebRequest`, `curl.exe`, or the manual ZIP install above.
- If ChatGPT OAuth sign-in fails, make sure nothing else is using `localhost:1455`.
- If Claude OAuth sign-in fails, make sure nothing else is using `localhost:45454`.
- If an API card shows an admin-access status, verify the key has org usage access and that Anthropic uses an `sk-ant-admin...` key.
- If the widget shows a provider load failure, verify the API key or reconnect the OAuth source.
- If secure storage is unavailable, UsageGuard will not save credentials.
