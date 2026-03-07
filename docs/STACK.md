# Tech Stack Plan (UsageGuard)

## Why this stack
- **Node.js + Electron + Commander**
- Runs on Windows/Linux/macOS from one codebase
- Gives both desktop app and CLI quickly
- Easy local-only distribution and setup

## Layers
1. `core/` (shared logic)
   - alerts engine
   - provider adapters interface/implementations
2. `apps/cli/`
   - `usageguard` command
3. `apps/desktop/`
   - Electron app using same core logic

## Distribution strategy
- CLI: `npm i -g usage-guard` (future publish)
- Desktop: Electron packaging (next step: electron-builder)

## Privacy model
- Local runtime only
- No required cloud DB
- Provider credentials stay on user machine

## Extensibility
- Add adapters for:
  - OpenAI API key usage
  - Anthropic API key usage
  - OAuth/runtime telemetry (provider-dependent)
