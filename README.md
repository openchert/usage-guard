# usage-guard

Local-first AI usage monitor with **desktop app + CLI**.

## What you get
- Cross-platform desktop app (Windows/macOS/Linux) via Electron
- CLI tool for scripts and terminal workflows
- Shared alert engine (near limit / exceeded limit / low usage)
- Provider adapter architecture for API key, OAuth, or runtime telemetry adapters
- Quiet-hours-aware notifications

## Quick start

### 1) Install
```bash
npm install
```

### 2) Run desktop app
```bash
npm start
```

### 3) Run CLI
```bash
node apps/cli/bin/usageguard.js demo
node apps/cli/bin/usageguard.js check --spent 12 --limit 20 --inactive-hours 9
node apps/cli/bin/usageguard.js config --near-limit-ratio 0.9 --quiet-start 23 --quiet-end 8
```

## Real provider data (current support)
UsageGuard supports three data paths today:

1. **Provider API path (when available)**
   - OpenAI via `OPENAI_API_KEY` and compatible `OPENAI_COSTS_ENDPOINT`
   - Anthropic via `ANTHROPIC_API_KEY` + optional `ANTHROPIC_COSTS_ENDPOINT`

2. **Local usage logs (recommended local-first path)**
   - `OPENAI_USAGE_LOG=/path/to/openai.ndjson`
   - `ANTHROPIC_USAGE_LOG=/path/to/anthropic.ndjson`

3. **Environment fallback values**
   - `OPENAI_SPENT_USD`, `OPENAI_LIMIT_USD`, etc.
   - `ANTHROPIC_SPENT_USD`, `ANTHROPIC_LIMIT_USD`, etc.

If none are configured, mock snapshots are shown for product validation.

## Validate locally
```bash
npm test
npm run cli:demo
npm run cli:check
```

## Packaging (desktop artifacts)
```bash
npm run build:win
npm run build:mac
npm run build:linux
```

## Project structure
- `core/` shared monitoring + alert + provider logic
- `apps/cli/` command-line tool
- `apps/desktop/` Electron desktop app
- `docs/` architecture + stack + setup + plan

See `docs/SETUP.md` for platform-specific run/build steps.

## Local-first privacy
- Runs locally on user machine
- No required cloud backend
- No mandatory remote storage
