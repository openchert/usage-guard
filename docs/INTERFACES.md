# Usage Interfaces (Provider Adapters)

UsageGuard normalizes every provider into this internal snapshot:

```json
{
  "provider": "string",
  "account_label": "string",
  "spent_usd": 0.0,
  "limit_usd": 0.0,
  "tokens_in": 0,
  "tokens_out": 0,
  "inactive_hours": 0,
  "source": "api|ndjson|env|demo|api-error:*"
}
```

## Adapter input priority (per provider)
1. `*_USAGE_LOG` (NDJSON, latest valid line)
2. API endpoint + API key
3. Env fallback (`*_SPENT_USD`, `*_LIMIT_USD`)
4. Demo fallback (if no providers produce data)

## Common JSON keys accepted (generic parser)
- Spend: `spent_usd`, `spent`, `cost_usd`, `total_cost_usd`
- Limit: `limit_usd`, `budget_usd`, `limit`, `hard_limit_usd`
- Input tokens: `tokens_in`, `input_tokens`, `total_input_tokens`
- Output tokens: `tokens_out`, `output_tokens`, `total_output_tokens`
- Inactivity: `inactive_hours` OR `last_activity_iso`/`last_activity`/`timestamp` (RFC3339)

## Secret storage interface
- `set_provider_api_key(provider_id, key)` stores keys in OS keyring when available.
- `get_provider_api_key(provider_id)` resolves keys from keyring.
- Config JSON may still hold legacy keys; UsageGuard migrates them to keyring on load when possible.

## Built-in providers

### OpenAI
- Key: `OPENAI_API_KEY` (or config `api.openai_api_key`)
- Endpoint: `OPENAI_COSTS_ENDPOINT` (default: `https://api.openai.com/v1/organization/costs`)
- Auth header: `Authorization: Bearer <key>`
- Log: `OPENAI_USAGE_LOG`

### Anthropic
- Key: `ANTHROPIC_API_KEY`
- Endpoint: `ANTHROPIC_COSTS_ENDPOINT` (default: `https://api.anthropic.com/v1/organizations/usage`)
- Headers:
  - `x-api-key: <key>`
  - `anthropic-version: 2023-06-01`
- Log: `ANTHROPIC_USAGE_LOG`

### Gemini
- Key: `GEMINI_API_KEY`
- Endpoint: `GEMINI_COSTS_ENDPOINT`
- Auth header: `Authorization: Bearer <key>`
- Log: `GEMINI_USAGE_LOG`

### Mistral
- Key: `MISTRAL_API_KEY`
- Endpoint: `MISTRAL_COSTS_ENDPOINT`
- Auth header: `Authorization: Bearer <key>`
- Log: `MISTRAL_USAGE_LOG`

### Groq
- Key: `GROQ_API_KEY`
- Endpoint: `GROQ_COSTS_ENDPOINT`
- Auth header: `Authorization: Bearer <key>`
- Log: `GROQ_USAGE_LOG`

### Together
- Key: `TOGETHER_API_KEY`
- Endpoint: `TOGETHER_COSTS_ENDPOINT`
- Auth header: `Authorization: Bearer <key>`
- Log: `TOGETHER_USAGE_LOG`

### OpenRouter
- Key: `OPENROUTER_API_KEY`
- Endpoint: `OPENROUTER_COSTS_ENDPOINT`
- Auth header: `Authorization: Bearer <key>`
- Log: `OPENROUTER_USAGE_LOG`

### Azure OpenAI
- Key: `AZURE_OPENAI_API_KEY`
- Endpoint: `AZURE_OPENAI_COSTS_ENDPOINT`
- Auth header: `api-key: <key>`
- Log: `AZURE_OPENAI_USAGE_LOG`

### Ollama
- Key: `OLLAMA_API_KEY` (optional, if your endpoint requires auth)
- Endpoint: `OLLAMA_USAGE_ENDPOINT`
- Auth header: `Authorization: Bearer <key>`
- Log: `OLLAMA_USAGE_LOG`

## Custom provider profiles (config)
`config.json` supports `profiles` to add any provider via endpoint + auth header:

```json
{
  "profiles": [
    {
      "id": "my_provider",
      "label": "My Provider",
      "endpoint": "https://example.com/usage",
      "auth_header": "Authorization",
      "api_key": "token"
    }
  ]
}
```

If `auth_header` is `Authorization`, UsageGuard sends `Bearer <api_key>`.
