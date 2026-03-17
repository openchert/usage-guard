# Security and Connection Model

## Scope

This document describes the current Windows and Linux implementation for secret storage, local consumer imports, and provider authentication.

## Secret storage

UsageGuard stores secrets through OS-native secure storage:

- Windows: DPAPI-encrypted blob at `%APPDATA%\usage-guard\secrets.bin`
- Linux: desktop Secret Service keyring through the OS credential store (for example GNOME Keyring or a KWallet-backed Secret Service provider)

For normal operation, the app reads and writes secrets only through secure OS storage for API keys and any legacy secrets from older builds. Consumer usage import reads local Codex and Claude Code files directly from the current user profile.

The encrypted payload currently contains:

- Provider API keys for OpenAI and Anthropic accounts
- Legacy OpenAI OAuth refresh token, `account_id`, and `plan_type` fields from earlier builds
- Legacy Anthropic OAuth refresh token, `subscription_type`, and `rate_limit_tier` fields from earlier builds

OAuth access tokens are not written to disk. The current default consumer flow does not request or refresh consumer OAuth tokens.

## Supported connection types

Current built-in connection types are:

- OpenAI `consumer_local`: Codex consumer usage via local Codex session logs under `~/.codex/sessions`
- Anthropic `consumer_local`: Claude Code exact `5h` quota via local CLI insights
- Anthropic `consumer_local_metrics`: Claude Code local token activity via `~/.claude/projects`
- Anthropic `consumer_local_status`: Claude Code local status via `~/.claude/.credentials.json`
- OpenAI `api`: `GET https://api.openai.com/v1/organization/costs` and `GET https://api.openai.com/v1/organization/usage/completions`
- Anthropic `api`: `GET https://api.anthropic.com/v1/organizations/cost_report` and `GET https://api.anthropic.com/v1/organizations/usage_report/messages`

Outbound requests are limited to these built-in audited endpoints. Custom endpoint overrides and custom provider profiles are not used for outbound fetches.

## OpenAI local consumer import

Codex consumer usage is imported from local files created by the official Codex client.

1. The official Codex client signs the user in locally.
2. UsageGuard detects local auth state from `~/.codex/auth.json`.
3. UsageGuard scans recent session logs under `~/.codex/sessions`.
4. The latest `token_count` event with `rate_limits` becomes the normalized `5h` and `week` quota snapshot.

No consumer OAuth token exchange is performed in the default UI path.

## Anthropic local consumer detection

Claude Code local state is read from the user profile.

1. The official Claude Code client signs the user in locally.
2. UsageGuard reads `subscriptionType` and `rateLimitTier` from `%USERPROFILE%\.claude\.credentials.json`.
3. UsageGuard runs `claude -p --verbose --output-format stream-json "/insights"` and parses the local `five_hour` quota event.
4. UsageGuard scans recent JSONL project logs under `%USERPROFILE%\.claude\projects`.
5. Recent assistant-message token counts are aggregated into rolling local activity windows.
6. The widget shows exact local `5h` quota and local `7d` activity.

UsageGuard does not import Claude access tokens from that file for normal operation. Weekly consumer quota percentage is not exposed through the local Claude Code client, so the built-in view shows exact `5h` quota plus local activity instead of a fake weekly percentage.

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
- accidental plaintext token leakage from local app files

It does not attempt to defend against:

- same-user malware
- a fully compromised desktop session
- an attacker who can already call the platform credential APIs as the logged-in user

## Current limitations

- Secure persistence is implemented on Windows and Linux in this release.
- If secure persistence is unavailable, UsageGuard does not intentionally fall back to plaintext secret storage.
- On Linux, a Secret Service provider must be available and unlocked for credential persistence to work.
- Claude Code local detection currently does not include a reliable local weekly consumer quota percentage.
