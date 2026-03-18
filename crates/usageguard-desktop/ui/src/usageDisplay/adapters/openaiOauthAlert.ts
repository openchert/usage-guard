import type {
  UsageCardSpec,
  UsageDisplayAdapter,
  UsageDisplayContext,
  UsageSnapshot,
} from '../types';
import { buildCardTitle, formatResetTime, quotaResetAt, quotaUsedPercent } from './shared';

const OAUTH_ERROR_CODES = new Set(['oauth_reauth_required', 'oauth_usage_unavailable']);

function clampRatio(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}

function remainingRatio(usedPercent: number | null): number {
  if (usedPercent == null || !Number.isFinite(usedPercent)) return 0;
  return clampRatio(1 - usedPercent / 100);
}

function displayLabel(snapshot: UsageSnapshot, context: UsageDisplayContext): string {
  return snapshot.account_label?.trim() || context.providerLabel;
}

function buildErrorStatusCard(
  snapshot: UsageSnapshot,
  context: UsageDisplayContext,
): UsageCardSpec {
  const label = displayLabel(snapshot, context);
  const isReauth = snapshot.status_code === 'oauth_reauth_required';

  const titleLines = [label];
  if (isReauth) {
    titleLines.push('Sign-in expired');
    titleLines.push('Open settings to re-authenticate');
  } else {
    titleLines.push('Usage data unavailable');
    if (snapshot.status_message) titleLines.push(snapshot.status_message);
    titleLines.push('Check connection or try refreshing');
  }

  return {
    kind: 'status',
    displayLabel: label,
    title: buildCardTitle(snapshot, titleLines),
    statusKind: isReauth ? 'auth' : 'error',
    label: isReauth ? 'sign in' : 'unavailable',
  };
}

export const openaiOauthDisplayAdapter: UsageDisplayAdapter = {
  id: 'openai-oauth',
  matches(snapshot) {
    return snapshot.provider === 'openai'
      && (snapshot.source === 'oauth' || snapshot.source === 'consumer_local');
  },
  toCard(snapshot, context): UsageCardSpec {
    if (snapshot.status_code && OAUTH_ERROR_CODES.has(snapshot.status_code)) {
      return buildErrorStatusCard(snapshot, context);
    }

    const label = displayLabel(snapshot, context);
    const primaryUsed = quotaUsedPercent(snapshot, 'primary') ?? 0;
    const secondaryUsed = quotaUsedPercent(snapshot, 'secondary') ?? 0;
    const primaryLeft = Math.round(remainingRatio(primaryUsed) * 100);
    const secondaryLeft = Math.round(remainingRatio(secondaryUsed) * 100);

    const primaryReset = formatResetTime(quotaResetAt(snapshot, 'primary'));
    const secondaryReset = formatResetTime(quotaResetAt(snapshot, 'secondary'));
    const titleLines = [label];
    titleLines.push(`5h used: ${primaryUsed}% | left: ${primaryLeft}%`);
    if (primaryReset) titleLines.push(`  resets: ${primaryReset}`);
    titleLines.push('---------------------');
    titleLines.push(`week used: ${secondaryUsed}% | left: ${secondaryLeft}%`);
    if (secondaryReset) titleLines.push(`  resets: ${secondaryReset}`);
    if (snapshot.status_message) {
      titleLines.push(`Status: ${snapshot.status_message}`);
    }

    return {
      kind: 'quota',
      displayLabel: label,
      title: buildCardTitle(snapshot, titleLines),
      rings: [
        { label: '5h', ratio: remainingRatio(primaryUsed) },
        { ratio: remainingRatio(secondaryUsed) },
      ],
    };
  },
};
