# usage-guard

Local-first, open-source AI usage monitor for OpenAI/Anthropic and runtime adapters.

## Goals
- Run fully on user machine (Windows, macOS, Linux)
- CLI-first + local web dashboard
- Support API key providers now, OAuth/runtime adapters pluggable
- Alert when usage is near limit **or** suspiciously low/inactive
- No cloud storage required

## Current Prototype
- Cross-platform Python CLI (`usageguard`)
- Local dashboard server (optional)
- Alert engine (limit threshold + inactivity threshold)

## Quickstart
```bash
python -m venv .venv
source .venv/bin/activate  # Windows: .venv\\Scripts\\activate
pip install -e .

usageguard demo
usageguard check --limit-usd 20 --spent-usd 14 --inactive-hours 10 --inactive-threshold-hours 8
usageguard dashboard
```

## CLI commands
- `usageguard demo` — show local usage snapshot and alerts
- `usageguard check` — evaluate usage against budgets/thresholds
- `usageguard dashboard` — run local dashboard server on localhost

## Provider support model
- **API key mode**: first-class and reliable
- **OAuth mode**: adapter-based, provider capability dependent
- **Runtime mode**: adapter-based

## Privacy model
- Local processing by default
- No required cloud DB
- Easy to run ephemeral/no-persist workflows

## Roadmap
See [docs/PLAN.md](docs/PLAN.md).
