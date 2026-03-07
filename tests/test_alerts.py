from usageguard.monitor import UsageSnapshot, evaluate_alerts


def test_near_limit_alert():
    s = UsageSnapshot("x", "y", spent_usd=9, limit_usd=10, tokens_in=0, tokens_out=0, inactive_hours=1)
    alerts = evaluate_alerts(s)
    assert any(a.code == "near_limit" for a in alerts)
