import type {
  UsageCardSpec,
  UsageDisplayAdapter,
  UsageDisplayContext,
  UsageSnapshot,
} from '../types';

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

    const titleLines = [
      label,
      'Claude OAuth',
      `session used: ${sessionUsed}% | left: ${sessionLeft}%`,
      `week used: ${weekUsed}% | left: ${weekLeft}%`,
    ];
    if (snapshot.status_message) {
      titleLines.push(`Status: ${snapshot.status_message}`);
    }

    return {
      displayLabel: label,
      title: titleLines.join('\n'),
      rings: [
        { label: '5h', ratio: remainingRatio(snapshot.tokens_in) },
        { ratio: remainingRatio(snapshot.spent_usd) },
      ],
    };
  },
};
