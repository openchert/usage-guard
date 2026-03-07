from __future__ import annotations

import json
import typer
from rich import print
from fastapi import FastAPI
import uvicorn

from usageguard.monitor import UsageSnapshot, evaluate_alerts
from usageguard.providers.openai import OpenAIAdapter
from usageguard.providers.anthropic import AnthropicAdapter

app = typer.Typer(help="UsageGuard CLI")


def _render_snapshot(snapshot: UsageSnapshot) -> None:
    print(f"[bold]Provider:[/bold] {snapshot.provider} ({snapshot.account_label})")
    print(f"[bold]Spend:[/bold] ${snapshot.spent_usd:.2f} / ${snapshot.limit_usd:.2f}")
    print(f"[bold]Tokens:[/bold] in={snapshot.tokens_in:,} out={snapshot.tokens_out:,}")
    print(f"[bold]Inactive:[/bold] {snapshot.inactive_hours}h")


@app.command()
def demo() -> None:
    o = OpenAIAdapter().fetch_usage()
    a = AnthropicAdapter().fetch_usage()
    for snap in [o, a]:
        _render_snapshot(snap)
        for alert in evaluate_alerts(snap):
            print(f"- [{alert.level}] {alert.message}")
        print()


@app.command()
def check(
    limit_usd: float = typer.Option(...),
    spent_usd: float = typer.Option(...),
    inactive_hours: int = typer.Option(0),
    inactive_threshold_hours: int = typer.Option(8),
) -> None:
    snap = UsageSnapshot(
        provider="custom",
        account_label="local",
        spent_usd=spent_usd,
        limit_usd=limit_usd,
        tokens_in=0,
        tokens_out=0,
        inactive_hours=inactive_hours,
    )
    _render_snapshot(snap)
    alerts = evaluate_alerts(snap, inactive_threshold_hours=inactive_threshold_hours)
    if not alerts:
        print("No alerts")
    for alert in alerts:
        print(f"- [{alert.level}] {alert.code}: {alert.message}")




@app.command()
def dashboard(host: str = "127.0.0.1", port: int = 8787) -> None:
    api = FastAPI(title="UsageGuard Local Dashboard")

    @api.get("/health")
    def health() -> dict:
        return {"ok": True}

    @api.get("/snapshot")
    def snapshot() -> dict:
        snap = OpenAIAdapter().fetch_usage()
        alerts = [a.__dict__ for a in evaluate_alerts(snap)]
        return {"snapshot": snap.__dict__, "alerts": alerts}

    @api.get("/")
    def root() -> dict:
        return {
            "name": "UsageGuard",
            "message": "Local API running. Use /snapshot for current data.",
        }

    uvicorn.run(api, host=host, port=port)


if __name__ == "__main__":
    app()
