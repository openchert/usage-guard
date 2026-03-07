# Adapter Examples

This file shows minimal extension examples for providers not built into UsageGuard.

## 1) Custom profile in config.json

Add to your config `profiles` array:

```json
{
  "id": "my_provider",
  "label": "My Provider",
  "endpoint": "https://example.com/usage",
  "auth_header": "Authorization",
  "api_key": "YOUR_TOKEN"
}
```

Reference file: `examples/providers/custom-provider.json`

## 2) NDJSON ingestion

Set provider log env var to a file with one JSON object per line.

Example schema (latest valid line is used):
- `spent_usd`
- `limit_usd`
- `tokens_in`
- `tokens_out`
- `last_activity_iso` (RFC3339)

Reference file: `examples/providers/usage-log.ndjson`

## 3) Optional compact ETA signal

You can expose burn-rate environment variables to activate ETA-to-limit display in the desktop detail view:

- `OPENAI_BURN_USD_PER_HOUR`
- `ANTHROPIC_BURN_USD_PER_HOUR`
- `GEMINI_BURN_USD_PER_HOUR`
- etc. (`<PROVIDER>_BURN_USD_PER_HOUR`)

When present and > 0, UsageGuard displays `ETA to limit` in expanded provider details.
