# Provider and Display Model

## Provider/source modules

The desktop widget no longer hardcodes provider-specific usage math in the main view.

Instead, each consumer source defines its own display adapter under:

`crates/usageguard-desktop/ui/src/usageDisplay/adapters/`

Current adapters:

- `openaiConsumerQuota.ts`
- `anthropicConsumerHybrid.ts`
- `consumerStatus.ts`
- `shared.ts`

This matters because each source can represent usage differently:

- Codex local sessions return `used_percent` values from recent local `token_count` events
- Claude Code local usage returns normalized consumer windows from the built-in local usage endpoint
- some sources return quota windows, while status-only sources return connection state without quota data

Each adapter is responsible for converting those raw semantics into the normalized card UI and hover text.

## OpenAI local consumer display behavior

Codex local consumer import uses recent JSONL session logs under `~/.codex/sessions`, with a token-backed fallback sourced from `~/.codex/auth.json`.

Acquisition order:

1. Read the freshest `token_count` event with `rate_limits` from `~/.codex/sessions/**/*.jsonl`.
2. If that payload is fresh, normalize it directly into the `5h` and `week` consumer windows.
3. If local session data is stale or missing, call the built-in Codex consumer usage endpoint using the access token and account id from `~/.codex/auth.json`.
4. Cache that fallback snapshot briefly so the widget refresh loop can reuse it.

The widget reads:

- `rate_limits.primary.used_percent` for the `5h` ring
- `rate_limits.secondary.used_percent` for the `week` ring

The UI converts usage into remaining quota for the display rings:

- `23% used` becomes `77% left`
- `24% used` becomes `76% left`

## Anthropic local consumer behavior

Claude Code local import uses `%USERPROFILE%\.claude\.credentials.json` or `~/.claude/.credentials.json`.

The credentials file provides both local status metadata and, when available, the local access token used for the built-in consumer usage fetch.

Acquisition order:

1. Read `subscriptionType` and `rateLimitTier` from the local credentials file to derive the Claude Code account label.
2. Read `accessToken` and `expiresAt` from that same file.
3. If the token is still valid, call the built-in Claude Code usage endpoint and normalize the response into consumer quota windows.
4. Cache the normalized windows briefly so the desktop status path stays cheap and startup does not block on a live fetch.

The built-in local adapters render:

- the `5h` quota window when the provider response includes it
- an optional `week` window when the provider response includes one

If quota data is not available yet, the widget falls back to a `consumer_local_status` snapshot with the local plan label instead of a fake quota ring.

## Snapshot schema

`UsageSnapshot` keeps `source` as a stable origin instead of mixing error text into it.

Current source values used by the app are:

- `consumer_local`
- `consumer_local_status`

User-safe error state is carried separately in:

- `status_code`
- `status_message`
- `primary_reset_at` / `secondary_reset_at` for consumer quota reset timestamps when present

That keeps the UI and CLI readable without leaking raw upstream response bodies.

## Built-in fetch policy

Outbound HTTP fetches are limited to built-in audited consumer endpoints.

Current built-in remote fetch sources:

- OpenAI Codex consumer usage endpoint, reached with the local token from `~/.codex/auth.json` only when fresh session logs are unavailable
- Anthropic Claude Code consumer usage endpoint, reached with the local token from `~/.claude/.credentials.json`

Local consumer files remain as non-remote sources where applicable.
