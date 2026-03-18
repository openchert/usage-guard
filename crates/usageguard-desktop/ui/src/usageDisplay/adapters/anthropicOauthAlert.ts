import type {
  UsageCardSpec,
  UsageDisplayAdapter,
  UsageDisplayContext,
  UsageSnapshot,
} from '../types';
import { buildCardTitle, formatResetTime } from './shared';

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

export const anthropicOauthDisplayAdapter: UsageDisplayAdapter = {
  id: 'anthropic-oauth',
  matches(snapshot) {
    return snapshot.provider === 'anthropic' && snapshot.source === 'oauth';
  },
  toCard(snapshot, context): UsageCardSpec {
    if (snapshot.status_code && OAUTH_ERROR_CODES.has(snapshot.status_code)) {
      return buildErrorStatusCard(snapshot, context);
    }

    const label = displayLabel(snapshot, context);
    const sessionUsed = snapshot.tokens_in ?? 0;
    const weekUsed = snapshot.spent_usd ?? 0;
    const sessionLeft = Math.round(remainingRatio(sessionUsed) * 100);
    const weekLeft = Math.round(remainingRatio(weekUsed) * 100);

    const primaryReset = formatResetTime(snapshot.primary_reset_at);
    const secondaryReset = formatResetTime(snapshot.secondary_reset_at);
    const titleLines = [label];
    titleLines.push(`5h used: ${sessionUsed}% | left: ${sessionLeft}%`);
    if (primaryReset) titleLines.push(`  resets: ${primaryReset}`);
    titleLines.push('---------------------');
    titleLines.push(`week used: ${weekUsed}% | left: ${weekLeft}%`);
    if (secondaryReset) titleLines.push(`  resets: ${secondaryReset}`);
    if (snapshot.status_message) {
      titleLines.push(`Status: ${snapshot.status_message}`);
    }

    return {
      kind: 'quota',
      displayLabel: label,
      title: buildCardTitle(snapshot, titleLines),
      rings: [
        { label: '5h', ratio: remainingRatio(snapshot.tokens_in) },
        { ratio: remainingRatio(snapshot.spent_usd) },
      ],
    };
  },
};
