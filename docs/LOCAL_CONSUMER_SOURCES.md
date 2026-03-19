# Local Consumer Sources

UsageGuard no longer runs its own browser sign-in flow for consumer accounts.

Instead, it reads the local state already created by the official Codex and Claude Code clients on the current machine, then normalizes that data into the shared consumer quota model used by the widget and alerts.

## Core rules

- UsageGuard never stores consumer access tokens in its own secret store.
- Provider API keys for organization monitoring still go through OS-native secure storage.
- Consumer access tokens stay in the official client files and are only used in memory when UsageGuard needs to call a built-in consumer usage endpoint.
- If local consumer usage is not available yet, the widget shows a local status card instead of asking the user to sign in through UsageGuard.

## Codex / OpenAI consumer flow

UsageGuard reads two local Codex locations:

- `~/.codex/auth.json`
- `~/.codex/sessions/**/*.jsonl`

Acquisition order:

1. Detect local sign-in from `~/.codex/auth.json`.
2. Scan recent session logs under `~/.codex/sessions` for the newest `token_count` event with `rate_limits`.
3. If that session payload is fresh, use it directly for the `5h` and `week` quota windows.
4. If the session data is stale or missing, use the access token and account id from `auth.json` to call the built-in Codex consumer usage endpoint.
5. Cache that normalized fallback snapshot briefly so the widget and refresh loop do not hammer the upstream endpoint.
6. If Codex is signed in locally but no usable quota data exists yet, show a waiting status snapshot until the next Codex request or a successful fallback fetch.

This means fresh local session logs are always preferred. The token-backed fetch is only the fallback path.

## Claude Code / Anthropic consumer flow

UsageGuard reads Claude Code state from:

- `~/.claude/.credentials.json`

The credentials file provides:

- `subscriptionType`
- `rateLimitTier`
- `accessToken`
- `expiresAt`

Acquisition order:

1. Detect local Claude Code sign-in from `~/.claude/.credentials.json`.
2. Use `subscriptionType` and `rateLimitTier` to derive the local account label and status.
3. If a valid unexpired `accessToken` exists, call the built-in Claude Code usage endpoint with that token.
4. Cache the normalized quota windows briefly for the widget refresh loop and for cheap status checks during startup.
5. If quota data is not available yet, keep showing the local status snapshot instead of blocking startup.

The current Claude consumer view reliably exposes the `5h` window. A second longer window can be normalized when the upstream response includes it, but the desktop status/settings flow still treats Claude weekly support as unavailable.

## What gets persisted

UsageGuard persists:

- provider API keys for organization/admin monitoring accounts
- local config such as labels, alert toggles, quiet hours, and UI settings

UsageGuard does not persist:

- consumer access tokens from Codex or Claude Code
- consumer refresh tokens
- browser sign-in session payloads from older builds

Legacy browser-sign-in artifacts from older builds are ignored during load and removed on the next save.

## Why this model exists

This keeps consumer monitoring local-first without requiring UsageGuard to become a sign-in broker for provider consumer accounts.

The official local clients remain responsible for sign-in and token lifecycle. UsageGuard only reads their local state, performs built-in audited fetches when needed, and renders the normalized result.
