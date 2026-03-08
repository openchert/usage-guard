import type { UsageCardSpec, UsageDisplayContext, UsageSnapshot } from '../types';

function clampRatio(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}

function displayLabel(snapshot: UsageSnapshot, context: UsageDisplayContext): string {
  return snapshot.account_label?.trim() || context.providerLabel;
}

function formatUsd(value: number): string {
  return `$${value.toFixed(2)}`;
}

function formatCount(value: number): string {
  return new Intl.NumberFormat('en-US').format(value);
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
    displayLabel: label,
    title: lines.join('\n'),
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

  return lines.join('\n');
}
