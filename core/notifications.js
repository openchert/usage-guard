export function isQuietHour(date, quietHours) {
  if (!quietHours?.enabled) return false;
  const h = date.getHours();
  const start = Number(quietHours.startHour);
  const end = Number(quietHours.endHour);

  if (start === end) return false;
  if (start < end) return h >= start && h < end;
  return h >= start || h < end;
}

export function shouldNotifyNow({ alerts = [], quietHours, now = new Date() }) {
  if (!alerts.length) return false;
  const critical = alerts.some((a) => a.level === 'critical');
  if (critical) return true;
  return !isQuietHour(now, quietHours);
}
