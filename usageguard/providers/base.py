from __future__ import annotations

from abc import ABC, abstractmethod
from usageguard.monitor import UsageSnapshot


class ProviderAdapter(ABC):
    name: str

    @abstractmethod
    def fetch_usage(self) -> UsageSnapshot:
        """Return a usage snapshot for this provider/account."""
        raise NotImplementedError
