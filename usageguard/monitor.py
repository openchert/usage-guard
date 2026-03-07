from __future__ import annotations

from dataclasses import dataclass
from typing import List


@dataclass
class UsageSnapshot:
    provider: str
    account_label: str
    spent_usd: float
    limit_usd: float
    tokens_in: int
    tokens_out: int
    inactive_hours: int


@dataclass
class Alert:
    level: str
    code: str
    message: str


def evaluate_alerts(snapshot: UsageSnapshot, inactive_threshold_hours: int = 8, near_limit_ratio: float = 0.85) -> List[Alert]:
    alerts: List[Alert] = []
    ratio = 0.0 if snapshot.limit_usd <= 0 else snapshot.spent_usd / snapshot.limit_usd

    if ratio >= 1.0:
        alerts.append(Alert("critical", "limit_exceeded", f"Budget exceeded: ${snapshot.spent_usd:.2f}/${snapshot.limit_usd:.2f}"))
    elif ratio >= near_limit_ratio:
        alerts.append(Alert("warning", "near_limit", f"Near budget limit: ${snapshot.spent_usd:.2f}/${snapshot.limit_usd:.2f}"))

    if snapshot.inactive_hours >= inactive_threshold_hours:
        alerts.append(Alert("info", "under_used", f"Low usage: no activity for {snapshot.inactive_hours}h"))

    return alerts
