# usage-guard

Local-first AI usage monitor with **desktop app + CLI**.

## What you get
- Cross-platform desktop app (Windows/macOS/Linux) via Electron
- CLI tool for scripts and terminal workflows
- Shared alert engine (near limit / exceeded limit / low usage)
- Provider adapter architecture for API key, OAuth, or runtime telemetry adapters

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
```

## Project structure
- `core/` shared monitoring + alert logic
- `apps/cli/` command-line tool
- `apps/desktop/` Electron desktop app
- `docs/` architecture + stack + plan

## Local-first privacy
- Runs locally on user machine
- No required cloud backend
- No mandatory remote storage

## Notes
Current provider adapters are mock snapshots for MVP validation.
Next step is wiring real OpenAI/Anthropic usage ingestion in `core/providers.js`.
