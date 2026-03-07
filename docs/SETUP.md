# Setup (Windows / macOS / Linux)

## Prerequisites
- Node.js 22+
- npm 10+

## Clone and run
```bash
git clone https://github.com/openchert/usage-guard.git
cd usage-guard
npm install
npm test
npm start
```

## CLI usage
```bash
node apps/cli/bin/usageguard.js demo
node apps/cli/bin/usageguard.js check --spent 12 --limit 20 --inactive-hours 9
node apps/cli/bin/usageguard.js config --near-limit-ratio 0.9 --quiet-start 22 --quiet-end 7
```

## Real provider data options
UsageGuard supports local-first input paths.

### OpenAI
- `OPENAI_API_KEY` (optional API path)
- `OPENAI_COSTS_ENDPOINT` (optional override)
- `OPENAI_USAGE_LOG=/path/to/openai.ndjson` (recommended)
- `OPENAI_LIMIT_USD`, `OPENAI_SPENT_USD` fallback env values

### Anthropic
- `ANTHROPIC_API_KEY` + optional `ANTHROPIC_COSTS_ENDPOINT`
- `ANTHROPIC_USAGE_LOG=/path/to/anthropic.ndjson` (recommended)
- `ANTHROPIC_LIMIT_USD`, `ANTHROPIC_SPENT_USD` fallback env values

## Build desktop packages
```bash
npm run build:win
npm run build:mac
npm run build:linux
```

Build output goes to `dist/`.
