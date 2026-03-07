from __future__ import annotations

import os
from usageguard.monitor import UsageSnapshot
from usageguard.providers.base import ProviderAdapter


class AnthropicAdapter(ProviderAdapter):
    name = "anthropic"

    def __init__(self, account_label: str = "Anthropic", api_key: str | None = None) -> None:
        self.account_label = account_label
        self.api_key = api_key or os.getenv("ANTHROPIC_API_KEY", "")

    def fetch_usage(self) -> UsageSnapshot:
        return UsageSnapshot(
            provider=self.name,
            account_label=self.account_label,
            spent_usd=6.7,
            limit_usd=20.0,
            tokens_in=92_000,
            tokens_out=8_400,
            inactive_hours=11,
        )
