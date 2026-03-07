export function evaluateAlerts({ spent = 0, limit = 0, inactiveHours = 0, nearLimitRatio = 0.85, inactiveThresholdHours = 8 }) {
  const alerts = [];
  const ratio = limit > 0 ? spent / limit : 0;

  if (limit > 0 && ratio >= 1) {
    alerts.push({ level: 'critical', code: 'limit_exceeded', message: `Budget exceeded: $${spent.toFixed(2)} / $${limit.toFixed(2)}` });
  } else if (limit > 0 && ratio >= nearLimitRatio) {
    alerts.push({ level: 'warning', code: 'near_limit', message: `Near budget limit: $${spent.toFixed(2)} / $${limit.toFixed(2)}` });
  }

  if (inactiveHours >= inactiveThresholdHours) {
    alerts.push({ level: 'info', code: 'under_used', message: `Low usage: no activity for ${inactiveHours}h` });
  }

  return alerts;
}
