import type {
  UsageCardSpec,
  UsageDisplayAdapter,
  UsageDisplayContext,
  UsageSnapshot,
} from '../types';
import { buildCardTitle, formatResetTime } from './shared';

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

export const openaiOauthDisplayAdapter: UsageDisplayAdapter = {
  id: 'openai-oauth',
  matches(snapshot) {
    return snapshot.provider === 'openai' && snapshot.source === 'oauth';
  },
  toCard(snapshot, context): UsageCardSpec {
    const label = displayLabel(snapshot, context);
    const primaryUsed = snapshot.tokens_in ?? 0;
    const secondaryUsed = snapshot.spent_usd ?? 0;
    const primaryLeft = Math.round(remainingRatio(primaryUsed) * 100);
    const secondaryLeft = Math.round(remainingRatio(secondaryUsed) * 100);

    const primaryReset = formatResetTime(snapshot.primary_reset_at);
    const secondaryReset = formatResetTime(snapshot.secondary_reset_at);
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
        { label: '5h', ratio: remainingRatio(snapshot.tokens_in) },
        { ratio: remainingRatio(snapshot.spent_usd) },
      ],
    };
  },
};
