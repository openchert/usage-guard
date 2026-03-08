# Provider and Display Model

## Provider/source modules

The desktop widget no longer hardcodes provider-specific usage math in the main view.

Instead, each provider/source combination can define its own display adapter under:

`crates/usageguard-desktop/ui/src/usageDisplay/adapters/`

Examples:

- `openaiOauth.ts`
- `openaiApi.ts`
- `anthropicApi.ts`
- `shared.ts`

This matters because each source can represent usage differently:

- OpenAI OAuth returns `used_percent` from `wham/usage`
- API providers may return spend, limits, tokens, or custom shapes
- some sources report consumed quota, others report remaining quota

Each adapter is responsible for converting those raw semantics into the normalized card UI, hover text, and ring values.

## OpenAI OAuth display behavior

OpenAI OAuth uses `https://chatgpt.com/backend-api/wham/usage`.

The widget reads:

- `rate_limit.primary_window.used_percent` for the `5h` ring
- `rate_limit.secondary_window.used_percent` for the `week` ring

The UI then converts usage into remaining quota for the display rings:

- `23% used` becomes `77% left`
- `24% used` becomes `76% left`

The OpenAI OAuth hover text is adapter-specific and shows both used and remaining values.

## Snapshot schema

`UsageSnapshot` now keeps `source` as a stable origin instead of mixing error text into it.

Current source values are:

- `oauth`
- `api`
- `env`
- `demo`

User-safe error state is carried separately in:

- `status_code`
- `status_message`

That keeps the UI and CLI readable without leaking raw upstream response bodies.

## Built-in fetch policy

For the first hardened deploy, outbound HTTP fetches are limited to built-in audited endpoints.

That means:

- custom endpoint overrides are ignored and purged on config load
- legacy custom provider profiles are cleared and not used for outbound fetches
- the desktop provider picker exposes only providers with built-in supported endpoints

Current built-in remote fetch sources:

- OpenAI OAuth
- OpenAI API
- Anthropic API
- Cursor API

Environment/log fallbacks and demo data still exist as non-remote fallback paths where applicable.
