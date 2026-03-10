import type {
  UsageCardSpec,
  UsageDisplayAdapter,
  UsageDisplayContext,
  UsageSnapshot,
} from '../types';
import { formatResetTime } from './shared';

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

export const anthropicOauthDisplayAdapter: UsageDisplayAdapter = {
  id: 'anthropic-oauth',
  matches(snapshot) {
    return snapshot.provider === 'anthropic' && snapshot.source === 'oauth';
  },
  toCard(snapshot, context): UsageCardSpec {
    const label = displayLabel(snapshot, context);
    const sessionUsed = snapshot.tokens_in ?? 0;
    const weekUsed = snapshot.spent_usd ?? 0;
    const sessionLeft = Math.round(remainingRatio(sessionUsed) * 100);
    const weekLeft = Math.round(remainingRatio(weekUsed) * 100);

    const primaryReset = formatResetTime(snapshot.primary_reset_at);
    const secondaryReset = formatResetTime(snapshot.secondary_reset_at);
    const titleLines = [
      label,
      `5H used: ${sessionUsed}% | left: ${sessionLeft}%${primaryReset ? ` | resets: ${primaryReset}` : ''}`,
      `week used: ${weekUsed}% | left: ${weekLeft}%${secondaryReset ? ` | resets: ${secondaryReset}` : ''}`,
    ];
    if (snapshot.status_message) {
      titleLines.push(`Status: ${snapshot.status_message}`);
    }

    return {
      kind: 'quota',
      displayLabel: label,
      title: titleLines.join('\n'),
      rings: [
        { label: '5h', ratio: remainingRatio(snapshot.tokens_in) },
        { ratio: remainingRatio(snapshot.spent_usd) },
      ],
    };
  },
};
