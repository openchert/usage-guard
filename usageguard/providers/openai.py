from __future__ import annotations

import os
from usageguard.monitor import UsageSnapshot
from usageguard.providers.base import ProviderAdapter


class OpenAIAdapter(ProviderAdapter):
    name = "openai"

    def __init__(self, account_label: str = "OpenAI", api_key: str | None = None) -> None:
        self.account_label = account_label
        self.api_key = api_key or os.getenv("OPENAI_API_KEY", "")

    def fetch_usage(self) -> UsageSnapshot:
        # Prototype behavior:
        # In production, call provider usage/billing APIs when available,
        # otherwise derive from local request logs.
        return UsageSnapshot(
            provider=self.name,
            account_label=self.account_label,
            spent_usd=12.4,
            limit_usd=30.0,
            tokens_in=184_000,
            tokens_out=12_300,
            inactive_hours=2,
        )
