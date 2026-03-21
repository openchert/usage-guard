import type {
  ConsumerQuotaWindow,
  UsageSnapshot,
} from '../types';

function quotaWindow(
  snapshot: UsageSnapshot,
  window: 'primary' | 'secondary',
): ConsumerQuotaWindow | null {
  return snapshot.consumer_quota?.[window] ?? null;
}

export function quotaUsedPercent(
  snapshot: UsageSnapshot,
  window: 'primary' | 'secondary',
): number | null {
  const quota = quotaWindow(snapshot, window);
  if (quota) {
    if (!quota.available) return null;
    return quota.used_percent ?? null;
  }

  if (window === 'primary') {
    return snapshot.tokens_in ?? null;
  }

  const hasLegacyWeekWindow = (snapshot.limit_usd ?? 0) > 0
    || (snapshot.spent_usd ?? 0) > 0
    || snapshot.secondary_reset_at != null;
  return hasLegacyWeekWindow ? (snapshot.spent_usd ?? null) : null;
}

export function quotaResetAt(
  snapshot: UsageSnapshot,
  window: 'primary' | 'secondary',
): string | null {
  const quota = quotaWindow(snapshot, window);
  if (quota) return quota.reset_at ?? null;
  return window === 'primary'
    ? (snapshot.primary_reset_at ?? null)
    : (snapshot.secondary_reset_at ?? null);
}

export function quotaWindowAvailable(
  snapshot: UsageSnapshot,
  window: 'primary' | 'secondary',
): boolean {
  const quota = quotaWindow(snapshot, window);
  if (quota) return quota.available && quota.used_percent != null;
  return quotaUsedPercent(snapshot, window) != null;
}

export function formatResetTime(value?: string | null): string | null {
  if (!value) return null;

  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;

  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(date);
}

function alertTitleLines(snapshot: UsageSnapshot): string[] {
  return (snapshot.alerts ?? []).map((alert) => `[${alert.level.toUpperCase()}] ${alert.message}`);
}

export function buildCardTitle(snapshot: UsageSnapshot, lines: string[]): string {
  const alerts = alertTitleLines(snapshot);
  return [...alerts, ...(alerts.length > 0 ? [''] : []), ...lines].join('\n');
}
