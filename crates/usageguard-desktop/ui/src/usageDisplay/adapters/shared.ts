import type {
  ApiMetricWindow,
  MetricStatSpec,
  UsageCardSpec,
  UsageDisplayContext,
  UsageSnapshot,
} from '../types';

function clampRatio(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}

function displayLabel(snapshot: UsageSnapshot, context: UsageDisplayContext): string {
  return snapshot.account_label?.trim() || context.providerLabel;
}

function formatUsd(value: number): string {
  return new Intl.NumberFormat('en-US', {
    style: 'currency',
    currency: 'USD',
    notation: Math.abs(value) >= 1000 ? 'compact' : 'standard',
    maximumFractionDigits: Math.abs(value) >= 100 ? 0 : 2,
  }).format(value);
}

function formatCount(value: number): string {
  return new Intl.NumberFormat('en-US').format(value);
}

function formatCompactCount(value: number): string {
  return new Intl.NumberFormat('en-US', {
    notation: value >= 1000 ? 'compact' : 'standard',
    maximumFractionDigits: value >= 1000 ? 1 : 0,
  }).format(value);
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

export function appendResetLine(lines: string[], label: string, value?: string | null): void {
  const formatted = formatResetTime(value);
  if (formatted) {
    lines.push(`${label} resets: ${formatted}`);
  }
}

function alertTitleLines(snapshot: UsageSnapshot): string[] {
  return (snapshot.alerts ?? []).map((alert) => `[${alert.level.toUpperCase()}] ${alert.message}`);
}

export function buildCardTitle(snapshot: UsageSnapshot, lines: string[]): string {
  const alerts = alertTitleLines(snapshot);
  return [...alerts, ...(alerts.length > 0 ? [''] : []), ...lines].join('\n');
}

function weeklyRatio(snapshot: UsageSnapshot): number {
  if (snapshot.limit_usd != null && snapshot.limit_usd > 0 && snapshot.spent_usd != null) {
    return clampRatio(snapshot.spent_usd / snapshot.limit_usd);
  }

  const total = (snapshot.tokens_in ?? 0) + (snapshot.tokens_out ?? 0);
  if (total > 0) return clampRatio(total / 1_200_000);
  return 0;
}

function shortWindowRatio(snapshot: UsageSnapshot): number {
  const weekly = weeklyRatio(snapshot);
  const inactive = snapshot.inactive_hours ?? 0;
  const activeFactor = Math.max(0.2, 1 - Math.min(inactive, 24) / 24);
  return clampRatio(weekly * (0.22 + activeFactor * 0.35));
}

export function buildGenericApiCard(
  snapshot: UsageSnapshot,
  context: UsageDisplayContext,
): UsageCardSpec {
  const label = displayLabel(snapshot, context);
  const lines = [label];

  if (snapshot.limit_usd > 0) {
    lines.push(`Spend: ${formatUsd(snapshot.spent_usd)} / ${formatUsd(snapshot.limit_usd)}`);
  } else if (snapshot.spent_usd > 0) {
    lines.push(`Spend: ${formatUsd(snapshot.spent_usd)}`);
  }

  const totalTokens = snapshot.tokens_in + snapshot.tokens_out;
  if (totalTokens > 0) {
    lines.push(`Tokens: in ${formatCount(snapshot.tokens_in)} | out ${formatCount(snapshot.tokens_out)}`);
  }

  if (snapshot.inactive_hours > 0) {
    lines.push(`Inactive: ${snapshot.inactive_hours}h`);
  }

  if (snapshot.status_message) {
    lines.push(`Status: ${snapshot.status_message}`);
  }

  return {
    kind: 'quota',
    displayLabel: label,
    title: buildCardTitle(snapshot, lines),
    rings: [
      { label: '5h', ratio: shortWindowRatio(snapshot) },
      { ratio: weeklyRatio(snapshot) },
    ],
  };
}

export function buildProviderApiTitle(
  providerName: string,
  snapshot: UsageSnapshot,
  context: UsageDisplayContext,
): string {
  const label = displayLabel(snapshot, context);
  const lines = [label, `${providerName} API`];

  if (snapshot.limit_usd > 0) {
    lines.push(`Spend: ${formatUsd(snapshot.spent_usd)} / ${formatUsd(snapshot.limit_usd)}`);
  } else if (snapshot.spent_usd > 0) {
    lines.push(`Spend: ${formatUsd(snapshot.spent_usd)}`);
  }

  const hasInput = snapshot.tokens_in > 0;
  const hasOutput = snapshot.tokens_out > 0;
  if (hasInput || hasOutput) {
    lines.push(`Tokens: in ${formatCount(snapshot.tokens_in)} | out ${formatCount(snapshot.tokens_out)}`);
  }

  if (snapshot.inactive_hours > 0) {
    lines.push(`Inactive: ${snapshot.inactive_hours}h`);
  }

  if (snapshot.status_message) {
    lines.push(`Status: ${snapshot.status_message}`);
  }

  return buildCardTitle(snapshot, lines);
}

function metricDetail(window?: ApiMetricWindow | null): string {
  if (!window) return 'No data';

  const parts = [];
  const hasTokens = window.tokens_in > 0 || window.tokens_out > 0;
  if (hasTokens) {
    parts.push(`In ${formatCompactCount(window.tokens_in)}`);
    parts.push(`Out ${formatCompactCount(window.tokens_out)}`);
  }
  if (window.requests != null) {
    parts.push(`${formatCompactCount(window.requests)} req`);
  }
  return parts.join(' · ') || 'No activity';
}

function metricStat(label: string, window?: ApiMetricWindow | null): MetricStatSpec {
  if (!window) {
    return {
      label,
      value: '--',
      detail: 'Unavailable',
    };
  }

  return {
    label,
    value: formatUsd(window.spend_usd),
    detail: metricDetail(window),
  };
}

function metricTitleLines(label: string, windowLabel: string, window?: ApiMetricWindow | null): string[] {
  if (!window) {
    return [`${windowLabel} spend: unavailable`, `${windowLabel} usage: unavailable`];
  }

  const lines = [`${windowLabel} spend: ${formatUsd(window.spend_usd)}`];
  if (window.tokens_in > 0 || window.tokens_out > 0) {
    lines.push(
      `${windowLabel} tokens: in ${formatCount(window.tokens_in)} | out ${formatCount(window.tokens_out)}`,
    );
  } else {
    lines.push(`${windowLabel} tokens: none`);
  }
  if (window.requests != null) {
    lines.push(`${windowLabel} requests: ${formatCount(window.requests)}`);
  }
  return lines;
}

export function buildProviderApiMetricCard(
  providerName: string,
  snapshot: UsageSnapshot,
  context: UsageDisplayContext,
  details: {
    spendSource: string;
    tokenSource: string;
  },
): UsageCardSpec {
  const label = displayLabel(snapshot, context);
  const metrics = snapshot.api_metrics;
  const titleLines = [
    label,
    `${providerName} Admin API`,
    ...metricTitleLines(label, 'Today', metrics?.today),
    ...metricTitleLines(label, '30d', metrics?.rolling_30d),
    `Spend source: ${details.spendSource}`,
    `Token source: ${details.tokenSource}`,
  ];

  if (snapshot.status_message) {
    titleLines.push(`Status: ${snapshot.status_message}`);
  }

  return {
    kind: 'metrics',
    displayLabel: label,
    title: buildCardTitle(snapshot, titleLines),
    stats: [
      metricStat('Today', metrics?.today),
      metricStat('30d', metrics?.rolling_30d),
    ],
  };
}
