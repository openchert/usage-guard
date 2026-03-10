<p align="center">
  <img src="public/assets/title.png" alt="UsageGuard" width="480">
</p>

<h1 align="center">UsageGuard</h1>
<p align="center">A local-first Windows widget and CLI for tracking AI spend, quotas, and subscription usage without dashboard noise.</p>
<p align="center"><strong>Windows x64 | PowerShell one-line install | No Rust required</strong></p>

UsageGuard keeps provider usage visible in a small desktop widget instead of burying it across multiple dashboards. It runs locally on Windows and stores your credentials securely on your machine.

## What It Does
- Shows ChatGPT and Claude subscription usage in one place
- Shows OpenAI and Anthropic organization/admin API monitoring in the same widget
- Displays one compact card per connected source, with hover details for deeper usage data
- Alerts you when a 5h or weekly subscription quota is nearly used up
- Reminds you to use remaining quota before a reset when usage is still low
- Supports browser sign-in for ChatGPT and Claude subscriptions
- Supports multiple organization or admin monitoring accounts for OpenAI and Anthropic
- Stores API keys and refresh tokens securely on Windows
- Includes both a desktop widget and an optional CLI

## Desktop Widget Features
- Compact floating widget with one card per connected provider or account
- Hover over any card to see detailed usage information
- Subscription card hover shows `5h` and `week` usage, remaining percentage, next reset times, and current status when available
- API/admin card hover shows `Today` and `30d` spend, token counts, request counts when available, metric source details, and current status
- Active alerts tint the card border, add an in-card badge, and prepend alert text to the hover details
- Native desktop notifications for important quota, budget, and inactivity alerts
- Drag the widget anywhere on screen; its position is restored on the next launch
- Widget width automatically grows or shrinks based on how many cards are connected
- Auto-refreshes in the background, with a manual `Refresh` action in the widget menu
- Right-click widget menu includes `Refresh`, `Manage Providers...`, `Always on Top`, `Light Mode`, `Hide to Tray`, and `Quit`
- Tray icon left-click toggles show/hide for the widget
- Tray icon menu includes `Show / Hide`, `Manage Providers...`, and `Quit UsageGuard`

## Provider Settings
- Open the provider window from the widget `+` button, the widget right-click menu, or the tray menu
- Connect ChatGPT with browser sign-in
- Connect Claude with browser sign-in
- Rename connected ChatGPT and Claude entries directly in the settings window
- Disconnect ChatGPT or Claude at any time
- Add multiple OpenAI organization monitoring accounts
- Add multiple Anthropic admin monitoring accounts
- Edit API account names without re-entering the stored key
- Replace an existing API key, or leave the key field blank to keep the current one while editing
- Remove API accounts with a confirmation step
- Verifies supported OpenAI organization keys and Anthropic `sk-ant-admin...` keys before saving them

## Install
### One-line Windows install
The installer downloads the latest Windows release from GitHub, extracts the binaries, and adds them to your user `PATH`.

```powershell
irm https://raw.githubusercontent.com/openchert/usage-guard/main/install.ps1 | iex
```

### Manual Windows install
1. Download `usage-guard-windows-x64.zip` from GitHub Releases.
2. Extract the archive.
3. Run `usageguard-desktop.exe` for the widget or `usageguard.exe` for the CLI.

### Verify release integrity
Each release includes `SHA256SUMS`.

```powershell
Get-FileHash .\usage-guard-windows-x64.zip -Algorithm SHA256
```

Compare the reported hash with the matching entry in `SHA256SUMS`.

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
- Use-before-reset reminders fire when `5h` resets within `45m` and usage is `<= 20%`, or when `week` resets within `24h` and usage is `<= 40%`
- API/admin monitoring sources keep the existing spend and inactivity alerts

See [`docs/ALERTS.md`](docs/ALERTS.md) for the full alert model and delivery behavior.

## Quick Start
### Desktop widget
1. Launch `usageguard-desktop`.
2. Click the `+` button on the widget, or right-click the widget.
3. Choose **Manage Providers...**
4. Connect ChatGPT or Claude, or add an OpenAI or Anthropic monitoring account with an API key.
5. Hover any provider card to inspect detailed usage, reset times, spend, token, or request data.
6. Right-click the widget for `Light Mode`, `Always on Top`, `Refresh`, and tray controls.
7. Keep the widget running to receive native notifications and card badges when an alert becomes active.

### Optional CLI
```bash
usageguard config --openai-key "sk-..."
usageguard config --anthropic-key "sk-ant-admin-..."
usageguard demo
```

## Security
On Windows, API keys and OAuth refresh tokens are stored in a DPAPI-encrypted file at `%APPDATA%\usage-guard\secrets.bin`. Access tokens stay in memory only and are refreshed when needed.

UsageGuard does not fall back to plaintext secret storage if secure storage is unavailable.

See [`docs/SECURITY.md`](docs/SECURITY.md) for storage, OAuth, and threat-model details.
See [`docs/ALERTS.md`](docs/ALERTS.md) for alert thresholds, native notifications, and widget badges.
See [`docs/PROVIDERS.md`](docs/PROVIDERS.md) for the provider/source display model.

## Troubleshooting
- If the install command succeeds but `usageguard` is not found, restart the terminal so `PATH` is reloaded.
- If ChatGPT OAuth sign-in fails, make sure nothing else is using `localhost:1455`.
- If Claude OAuth sign-in fails, make sure nothing else is using `localhost:45454`.
- If an OpenAI or Anthropic API card shows an admin-access status, verify the key has org usage access and that Anthropic uses an `sk-ant-admin...` key.
- If the widget shows `Status: Unable to load provider usage right now.`, verify the API key or reconnect the OAuth source.
- If secure storage is unavailable, UsageGuard will not save credentials.
