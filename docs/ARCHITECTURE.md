# Architecture

## Components
- `usageguard/cli.py` — CLI and local dashboard entrypoints
- `usageguard/monitor.py` — core usage snapshot + alert logic
- `usageguard/providers/*` — provider adapters

## Data handling
- Local-only by default
- No required remote persistence
