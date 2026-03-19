# Security and Connection Model

## Scope

This document describes the current Windows and Linux implementation for secret storage, local consumer imports, and provider authentication.

## Secret storage

UsageGuard stores secrets through OS-native secure storage:

- Windows: DPAPI-encrypted blob at `%APPDATA%\usage-guard\secrets.bin`
- Linux: desktop Secret Service keyring through the OS credential store (for example GNOME Keyring or a KWallet-backed Secret Service provider)

For normal operation, the app reads and writes secrets only through secure OS storage for provider API keys. Consumer monitoring reads local Codex and Claude Code files directly from the current user profile and uses those local credentials only in memory when a built-in consumer usage fetch is needed.

The encrypted payload currently contains:

- Provider API keys for OpenAI and Anthropic accounts

Legacy browser-sign-in payloads from earlier builds are ignored and cleaned up during migration. The current consumer flow does not request, store, or refresh consumer tokens inside UsageGuard.

## Supported connection types

Current built-in connection types are:

- OpenAI `consumer_local`: Codex consumer usage via fresh local session logs under `~/.codex/sessions`, with a built-in fallback fetch that uses the local token from `~/.codex/auth.json`
- OpenAI `consumer_local_status`: Codex local signed-in status when quota data is not available yet
- Anthropic `consumer_local`: Claude Code consumer usage via the local credentials file plus a built-in usage fetch that uses the local token from `~/.claude/.credentials.json`
- Anthropic `consumer_local_status`: Claude Code local status via `~/.claude/.credentials.json` when quota data is not available yet
- OpenAI `api`: `GET https://api.openai.com/v1/organization/costs` and `GET https://api.openai.com/v1/organization/usage/completions`
- Anthropic `api`: `GET https://api.anthropic.com/v1/organizations/cost_report` and `GET https://api.anthropic.com/v1/organizations/usage_report/messages`

Outbound requests are limited to these built-in audited endpoints. Custom endpoint overrides and custom provider profiles are not used for outbound fetches.

## OpenAI local consumer import

Codex consumer usage is imported from local files created by the official Codex client.

1. The official Codex client signs the user in locally.
2. UsageGuard detects local sign-in and reads the local access token and account id from `~/.codex/auth.json`.
3. UsageGuard scans recent session logs under `~/.codex/sessions`.
4. The latest fresh `token_count` event with `rate_limits` becomes the normalized `5h` and `week` quota snapshot.
5. If the session logs are stale or missing, UsageGuard uses the local access token from `auth.json` in memory to call the built-in Codex consumer usage endpoint.
6. The fallback response is normalized into the same consumer quota model and cached briefly.

## Anthropic local consumer detection

Claude Code local state is read from the user profile.

1. The official Claude Code client signs the user in locally.
2. UsageGuard reads `subscriptionType`, `rateLimitTier`, `accessToken`, and `expiresAt` from `%USERPROFILE%\.claude\.credentials.json` or `~/.claude/.credentials.json`.
3. `subscriptionType` and `rateLimitTier` drive the local plan label shown by the widget.
4. If the local `accessToken` is still valid, UsageGuard uses it in memory to call the built-in Claude Code usage endpoint.
5. The response is normalized into the shared consumer quota model and cached briefly for the widget refresh loop.
6. If quota data is not available yet, the widget still shows the local Claude Code status snapshot instead of a browser sign-in prompt.

The current desktop status/settings flow officially treats Claude `5h` quota as supported. A longer secondary window can still appear in the widget when the upstream response includes it.

## API-key authentication

Built-in API providers use these authentication methods:

- OpenAI API: `Authorization: Bearer <api key>` with organization/admin usage access for the Administration endpoints
- Anthropic API: `x-api-key: <api key>` with `anthropic-version: 2023-06-01`; organization monitoring requires an Admin API key (`sk-ant-admin...`)

UsageGuard does not accept individual API keys for provider-reported historical usage. Individual-user monitoring is handled through local consumer imports where the official local client exposes enough data.

## UI and command hardening

The desktop app includes these protections:

- Tauri global bridge disabled (`withGlobalTauri: false`)
- restrictive CSP enabled in `tauri.conf.json`
- UI uses `@tauri-apps/api` imports instead of `window.__TAURI__`
- sensitive commands validate the calling window label, so mutating actions only run from the settings window

## Threat model

This protects secrets at rest against casual disclosure such as:

- copied config directories
- inspecting files in `%APPDATA%`
- copying Linux config directories without also having access to the user's unlocked secret-service session
- accidental plaintext token leakage from UsageGuard-managed storage

It does not attempt to defend against:

- same-user malware
- a fully compromised desktop session
- an attacker who can already call the platform credential APIs as the logged-in user

## Current limitations

- Secure persistence is implemented on Windows and Linux in this release.
- If secure persistence is unavailable, UsageGuard does not intentionally fall back to plaintext secret storage.
- On Linux, a Secret Service provider must be available and unlocked for credential persistence to work.
- Claude Code desktop status/settings currently expose the `5h` window as the supported local quota signal, even though the normalized snapshot can carry a longer secondary window when the upstream response includes it.
