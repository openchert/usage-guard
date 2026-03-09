# Security and Connection Model

## Scope

This document describes the current Windows implementation for secret storage and provider authentication.

## Secret storage

UsageGuard stores secrets in a DPAPI-encrypted blob at:

`%APPDATA%\usage-guard\secrets.bin`

For normal operation, the app reads and writes secrets only through this encrypted store. It does not use plaintext JSON files for active OAuth sessions.

The encrypted payload currently contains:

- Provider API keys for OpenAI, Anthropic, and Cursor accounts
- OpenAI OAuth refresh token
- OpenAI OAuth `account_id`
- OpenAI OAuth `plan_type`
- Anthropic OAuth refresh token
- Anthropic OAuth `subscription_type`
- Anthropic OAuth `rate_limit_tier`

OAuth access tokens are not written to disk. They remain in memory and are refreshed when needed.

## Supported connection types

Current built-in connection types are:

- OpenAI `oauth`: ChatGPT subscription usage via `https://chatgpt.com/backend-api/wham/usage`
- Anthropic `oauth`: Claude subscription usage via `https://api.anthropic.com/api/oauth/usage`
- OpenAI `api`: `GET https://api.openai.com/v1/organization/costs`
- Anthropic `api`: `GET https://api.anthropic.com/v1/organizations/usage`
- Cursor `api`: `POST https://api.cursor.com/teams/spend`

Outbound requests are limited to these built-in audited endpoints. Custom endpoint overrides and custom provider profiles are not used for outbound fetches.

## OpenAI OAuth flow

OpenAI sign-in is a browser-based PKCE flow.

1. The settings window starts sign-in.
2. UsageGuard opens the system browser to `https://auth.openai.com/oauth/authorize`.
3. OpenAI redirects back to `http://localhost:1455/auth/callback`.
4. UsageGuard accepts the callback only while a sign-in is active.
5. The callback path and OAuth `state` must match the expected values.
6. The app exchanges the authorization code at `https://auth.openai.com/oauth/token`.
7. The refresh token is persisted in `secrets.bin`; the access token stays in memory.
8. The app fetches `https://chatgpt.com/backend-api/wham/usage` and caches `account_id` and `plan_type`.

If token refresh fails in a way that indicates expired or invalid OAuth state, UsageGuard clears the stored OpenAI OAuth session and requires the user to reconnect.

## Anthropic OAuth flow

Anthropic sign-in is also a browser-based PKCE flow.

1. The settings window starts sign-in.
2. UsageGuard opens the system browser to `https://claude.ai/oauth/authorize`.
3. Anthropic redirects back to `http://localhost:45454/callback`.
4. UsageGuard requires the exact callback path and the expected OAuth `state`.
5. The app exchanges the authorization code at `https://platform.claude.com/v1/oauth/token`.
6. The refresh token is persisted in `secrets.bin`; the access token stays in memory.
7. The app fetches subscription usage from `https://api.anthropic.com/api/oauth/usage`.

If Anthropic's OAuth response does not include plan metadata, UsageGuard may read only `subscriptionType` and `rateLimitTier` from `%USERPROFILE%\.claude\.credentials.json` and cache those values in `secrets.bin`. It does not import access tokens from that file for normal operation.

If token refresh fails in a way that indicates expired or invalid OAuth state, UsageGuard clears the stored Anthropic OAuth session and requires the user to reconnect.

## API-key authentication

Built-in API providers use these authentication methods:

- OpenAI API: `Authorization: Bearer <api key>`
- Anthropic API: `x-api-key: <api key>` with `anthropic-version: 2023-06-01`
- Cursor API: HTTP Basic auth using the API key as the username

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
- accidental plaintext token leakage from local app files

It does not attempt to defend against:

- same-user malware
- a fully compromised Windows session
- an attacker who can already call DPAPI as the logged-in user

## Current limitations

- Secure persistence is implemented for Windows in this release.
- If secure persistence is unavailable, UsageGuard does not intentionally fall back to plaintext secret storage.
- If another process is already using port `1455`, OpenAI OAuth sign-in will fail until the port is free.
- If another process is already using port `45454`, Anthropic OAuth sign-in will fail until the port is free.
