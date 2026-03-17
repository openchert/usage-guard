import { buildCardTitle } from './shared';
import type {
  MetricStatSpec,
  UsageCardSpec,
  UsageDisplayAdapter,
  UsageDisplayContext,
  UsageSnapshot,
} from '../types';

function displayLabel(snapshot: UsageSnapshot, context: UsageDisplayContext): string {
  return snapshot.account_label?.trim() || context.providerLabel;
}

function statusStats(snapshot: UsageSnapshot): MetricStatSpec[] {
  const usageValue = snapshot.provider === 'openai' ? 'Soon' : 'Pending';
  const usageDetail = snapshot.provider === 'openai'
    ? 'Waiting for the first local Codex usage event'
    : 'Exact 5h quota unavailable; weekly quota is not exposed locally';

  return [
    {
      label: 'Source',
      value: 'Local',
      detail: snapshot.provider === 'openai' ? 'Codex' : 'Claude Code',
    },
    {
      label: 'Usage',
      value: usageValue,
      detail: usageDetail,
    },
  ];
}

export const consumerStatusDisplayAdapter: UsageDisplayAdapter = {
  id: 'consumer-status',
  matches(snapshot) {
    return snapshot.source === 'consumer_local_status';
  },
  toCard(snapshot, context): UsageCardSpec {
    const label = displayLabel(snapshot, context);
    const lines = [
      label,
      snapshot.provider === 'openai' ? 'Codex local client' : 'Claude Code local client',
    ];

    if (snapshot.status_message) {
      lines.push(`Status: ${snapshot.status_message}`);
    }

    return {
      kind: 'metrics',
      displayLabel: label,
      title: buildCardTitle(snapshot, lines),
      stats: statusStats(snapshot),
    };
  },
};
