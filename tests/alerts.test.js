import test from 'node:test';
import assert from 'node:assert/strict';
import { evaluateAlerts } from '../core/alerts.js';
import { isQuietHour, shouldNotifyNow } from '../core/notifications.js';

test('near limit alert appears', () => {
  const alerts = evaluateAlerts({ spent: 18, limit: 20, inactiveHours: 0 });
  assert.ok(alerts.some((a) => a.code === 'near_limit'));
});

test('quiet hours block non-critical notifications', () => {
  const now = new Date('2026-03-07T23:30:00');
  assert.equal(isQuietHour(now, { enabled: true, startHour: 23, endHour: 8 }), true);
  const ok = shouldNotifyNow({ alerts: [{ level: 'warning' }], quietHours: { enabled: true, startHour: 23, endHour: 8 }, now });
  assert.equal(ok, false);
});

test('critical bypasses quiet hours', () => {
  const now = new Date('2026-03-07T23:30:00');
  const ok = shouldNotifyNow({ alerts: [{ level: 'critical' }], quietHours: { enabled: true, startHour: 23, endHour: 8 }, now });
  assert.equal(ok, true);
});
