# Provider and Display Model

## Provider/source modules

The desktop widget no longer hardcodes provider-specific usage math in the main view.

Instead, each provider/source combination can define its own display adapter under:

`crates/usageguard-desktop/ui/src/usageDisplay/adapters/`

Examples:

- `openaiOauth.ts`
- `anthropicOauth.ts`
- `openaiApi.ts`
- `anthropicApi.ts`
- `shared.ts`

This matters because each source can represent usage differently:

- OpenAI OAuth returns `used_percent` from `wham/usage`
- API providers may return rolling cost and usage aggregates instead of quota windows
- some sources report consumed quota, others report remaining quota

Each adapter is responsible for converting those raw semantics into the normalized card UI, hover text, and either quota rings or metric tiles.

## OpenAI OAuth display behavior

OpenAI OAuth uses `https://chatgpt.com/backend-api/wham/usage`.

The widget reads:

- `rate_limit.primary_window.used_percent` for the `5h` ring
- `rate_limit.secondary_window.used_percent` for the `week` ring

The UI then converts usage into remaining quota for the display rings:

- `23% used` becomes `77% left`
- `24% used` becomes `76% left`

The OpenAI OAuth hover text is adapter-specific and shows both used and remaining values.

## Anthropic OAuth display behavior

Anthropic OAuth uses `https://api.anthropic.com/api/oauth/usage`.

The widget reads:

- the five-hour utilization bucket for the `5h` ring
- the seven-day utilization bucket for the `week` ring

The Anthropic OAuth adapter also shows provider-specific hover text with:

- current five-hour usage and remaining percentage
- current seven-day usage and remaining percentage

## API display behavior

OpenAI API and Anthropic API cards do not reuse the OAuth `5h` / `week` quota model.

Instead, they render compact metric panels with:

- `Today` spend and activity
- rolling `30d` spend and activity
- token counts in tooltips
- request counts when the provider exposes them

These API cards are organization/admin monitoring only. Individual API keys are not supported for this built-in provider-reported view.

Current provider mappings:

- OpenAI API:
  `organization/costs` drives spend
  `organization/usage/completions` drives token and request counts
- Anthropic API:
  `organizations/cost_report` drives spend
  `organizations/usage_report/messages` drives token counts

The API hover text also explains the upstream source for each metric so billing-style spend and usage-report tokens are not conflated.

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
- `api_metrics` for typed `Today` / `30d` API card data when present

That keeps the UI and CLI readable without leaking raw upstream response bodies.

## Built-in fetch policy

For the first hardened deploy, outbound HTTP fetches are limited to built-in audited endpoints.

That means:

- custom endpoint overrides are ignored and purged on config load
- legacy custom provider profiles are cleared and not used for outbound fetches
- the desktop provider picker exposes only providers with built-in supported endpoints

Current built-in remote fetch sources:

- OpenAI OAuth
- Anthropic OAuth
- OpenAI API
- Anthropic API

Environment/log fallbacks and demo data still exist as non-remote fallback paths where applicable.
