import { buildCardTitle, formatResetTime, quotaResetAt, quotaUsedPercent } from './shared';
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

export const anthropicConsumerHybridDisplayAdapter: UsageDisplayAdapter = {
  id: 'anthropic-consumer-hybrid',
  matches(snapshot) {
    return snapshot.provider === 'anthropic'
      && snapshot.source === 'consumer_local'
      && quotaUsedPercent(snapshot, 'primary') != null;
  },
  toCard(snapshot, context): UsageCardSpec {
    const label = displayLabel(snapshot, context);
    const primaryUsed = quotaUsedPercent(snapshot, 'primary') ?? 0;
    const primaryLeft = Math.round(remainingRatio(primaryUsed) * 100);
    const primaryReset = formatResetTime(quotaResetAt(snapshot, 'primary'));

    const secondaryUsed = quotaUsedPercent(snapshot, 'secondary');
    const hasWeekly = secondaryUsed != null;
    const secondaryLeft = hasWeekly ? Math.round(remainingRatio(secondaryUsed) * 100) : null;
    const secondaryReset = formatResetTime(quotaResetAt(snapshot, 'secondary'));

    const titleLines = [
      label,
      'Claude Code local quota',
      `5h used: ${Math.round(primaryUsed)}% | left: ${primaryLeft}%`,
    ];
    if (primaryReset) {
      titleLines.push(`5h resets: ${primaryReset}`);
    }
    if (hasWeekly) {
      titleLines.push('---------------------');
      titleLines.push(`week used: ${Math.round(secondaryUsed!)}% | left: ${secondaryLeft}%`);
      if (secondaryReset) {
        titleLines.push(`week resets: ${secondaryReset}`);
      }
    }

    const rings: UsageCardSpec['rings'] = [
      { label: '5h', ratio: remainingRatio(primaryUsed) },
    ];
    if (hasWeekly) {
      rings.push({ ratio: remainingRatio(secondaryUsed!) });
    }

    return {
      kind: 'quota',
      displayLabel: label,
      title: buildCardTitle(snapshot, titleLines),
      rings,
    };
  },
};
