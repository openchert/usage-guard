# Security and Connection Model

## Scope

This document describes the current Windows and Linux implementation for local consumer imports and built-in consumer usage fetches.

## Local data handling

UsageGuard no longer stores external provider credentials or runs an account-management flow for remote monitoring.

For normal operation:

- Codex state is read from local client files already present on the machine.
- Claude Code state is read from local client files already present on the machine.
- Local consumer access tokens stay in those client files and are only used in memory when a built-in consumer usage fetch is needed.
- UsageGuard persists only local app config such as labels, alert toggles, refresh interval, theme, and widget position.

Legacy provider-secret artifacts from older builds are ignored by the current runtime and are not required for normal operation.

## Supported connection types

Current built-in connection types are:

- OpenAI `consumer_local`: Codex consumer usage via fresh local session logs under `~/.codex/sessions`, with a built-in fallback fetch that uses the local token from `~/.codex/auth.json`
- OpenAI `consumer_local_status`: Codex local signed-in status when quota data is not available yet
- Anthropic `consumer_local`: Claude Code consumer usage via the local credentials file plus a built-in usage fetch that uses the local token from `~/.claude/.credentials.json`
- Anthropic `consumer_local_status`: Claude Code local status via `~/.claude/.credentials.json` when quota data is not available yet

Outbound requests are limited to these built-in consumer endpoints.

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

Claude Code local status and settings now expose weekly support whenever the normalized local snapshot contains a secondary quota window.

## UI and command hardening

The desktop app includes these protections:

- Tauri global bridge disabled (`withGlobalTauri: false`)
- restrictive CSP enabled in `tauri.conf.json`
- UI uses `@tauri-apps/api` imports instead of `window.__TAURI__`
- sensitive commands validate the calling window label, so mutating actions only run from the connections window

## Threat model

This protects secrets at rest against casual disclosure such as:

- copied config directories
- inspecting files in `%APPDATA%`
- accidental plaintext leakage from UsageGuard-managed config files

It does not attempt to defend against:

- same-user malware
- a fully compromised desktop session
- an attacker who can already access the local Codex or Claude Code client files as the logged-in user

## Current limitations

- UsageGuard depends on the local Codex or Claude Code client already being signed in on the current machine.
- If the local client has not produced usable quota data yet, UsageGuard shows a status card until data becomes available.
- Claude Code weekly support depends on the upstream local usage response including a secondary window.
