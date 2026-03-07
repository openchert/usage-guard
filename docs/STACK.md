# Tech Stack Plan (UsageGuard)

## Why this stack
- **Node.js + Electron + Commander**
- Runs on Windows/Linux/macOS from one codebase
- Gives both desktop app and CLI with shared logic
- Easy local-only distribution and setup

## Layers
1. `core/` (shared logic)
   - alerts engine
   - config/quiet-hours
   - provider adapters
2. `apps/cli/`
   - `usageguard` command
3. `apps/desktop/`
   - Electron app using same core logic + desktop notifications

## Distribution strategy
- CLI: npm bin (`usageguard`)
- Desktop: electron-builder (`build:win`, `build:mac`, `build:linux`)

## Privacy model
- Local runtime only
- No required cloud DB
- Provider credentials remain on user machine

## Extensibility
- Add adapters for:
  - OpenAI API key usage
  - Anthropic API key usage
  - OAuth/runtime telemetry (provider-dependent)
  - local exported logs/CSV/JSON pipelines
