# Security and OAuth

## Secret storage

For the hardened Windows release, UsageGuard stores secrets in a DPAPI-encrypted blob at:

`%APPDATA%\usage-guard\secrets.bin`

The encrypted payload currently contains:

- Provider API keys
- OpenAI OAuth refresh token
- OpenAI OAuth `account_id`
- OpenAI OAuth `plan_type`

OpenAI OAuth access tokens are not written to disk. They stay in memory only and are refreshed when needed.

## Migration

On startup, the app migrates older secret locations into `secrets.bin`:

- `%APPDATA%\usage-guard\oauth_tokens.json`
- legacy keyring entries for provider API keys
- the legacy OpenAI OAuth keyring entry

After a successful migration, the old plaintext OAuth file and migrated keyring entries are deleted.

## OpenAI OAuth flow

The current OpenAI OAuth flow is a browser-based PKCE flow.

How it works:

1. The settings window starts sign-in.
2. UsageGuard opens the system browser to `auth.openai.com`.
3. OpenAI redirects back to `http://localhost:1455/auth/callback`.
4. UsageGuard accepts the callback only while a sign-in is active.
5. The callback must match the exact path and the expected OAuth `state`.
6. The app exchanges the authorization code for tokens.
7. The refresh token is persisted in `secrets.bin`; the access token remains in memory.
8. The app fetches `https://chatgpt.com/backend-api/wham/usage` and learns `account_id` and `plan_type` from that trusted response.

Why `localhost:1455` is fixed:

- OpenAI OAuth in this app is currently registered and known to work with the fixed localhost redirect.
- A random loopback redirect was more standards-aligned, but it broke compatibility with the OpenAI OAuth client used here.

## UI and command hardening

The desktop app now includes these protections:

- Tauri global bridge disabled (`withGlobalTauri: false`)
- restrictive CSP enabled in `tauri.conf.json`
- UI uses `@tauri-apps/api` imports instead of `window.__TAURI__`
- sensitive commands validate the calling window label, so mutating actions only run from the settings window

## Threat model

This protects secrets at rest against casual disclosure such as:

- copied config directories
- inspecting files in `%APPDATA%`
- accidental plaintext token leakage from local app files

It does not attempt to defend against:

- same-user malware
- a fully compromised Windows session
- an attacker who can already call DPAPI as the logged-in user

## Current limitations

- Secure persistence is implemented for Windows in this release.
- If secure persistence is unavailable, UsageGuard does not intentionally fall back to plaintext secret storage.
- If another process is already using port `1455`, OpenAI OAuth sign-in will fail until the port is free.
