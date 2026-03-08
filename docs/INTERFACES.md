# Usage Interfaces

This file is the short reference for the current normalized runtime interfaces.

For the provider display model, see `docs/PROVIDERS.md`.
For secure storage and OAuth flow details, see `docs/SECURITY.md`.

## Normalized snapshot

UsageGuard normalizes provider data into this shape:

```json
{
  "provider": "string",
  "account_label": "string",
  "spent_usd": 0.0,
  "limit_usd": 0.0,
  "tokens_in": 0,
  "tokens_out": 0,
  "inactive_hours": 0,
  "source": "oauth|api|env|demo",
  "status_code": "string|null",
  "status_message": "string|null"
}
```

`source` is a stable origin label, not a raw error carrier.

`status_code` and `status_message` hold bounded user-safe error state when a provider cannot be refreshed.

## Snapshot input priority

Per provider, data is resolved in this order:

1. provider-specific OAuth source, when applicable
2. provider API fetch using a built-in audited endpoint
3. environment or usage-log fallback, where supported
4. demo data fallback

## Secret storage interface

The current storage helpers are:

- `set_provider_api_key(provider_id, key)`
- `get_provider_api_key(provider_id)`
- `set_provider_account_api_key(account_id, key)`
- `get_provider_account_api_key(account_id)`

On Windows, these persist into the DPAPI-encrypted blob at `%APPDATA%\usage-guard\secrets.bin`.

Legacy plaintext OAuth storage and legacy keyring entries are migrated into that encrypted store on load.

## Named provider accounts

Desktop-managed accounts live in `provider_accounts` inside `config.json`:

```json
{
  "provider_accounts": [
    {
      "id": "acct_openai_work_123456",
      "provider": "openai",
      "label": "Work",
      "endpoint": null
    }
  ]
}
```

`endpoint` remains in the config shape for compatibility, but the hardened desktop fetch path does not use custom endpoint overrides.

## Built-in fetch policy

For the first hardened deploy:

- custom endpoint overrides are ignored and purged on load
- legacy custom provider profiles are not used for outbound fetches
- the desktop provider picker exposes only providers with built-in audited endpoints
