import { anthropicApiDisplayAdapter } from './adapters/anthropicApi';
import { anthropicConsumerHybridDisplayAdapter } from './adapters/anthropicConsumerHybrid';
import { consumerStatusDisplayAdapter } from './adapters/consumerStatus';
import { genericApiDisplayAdapter } from './adapters/genericApi';
import { openaiConsumerQuotaDisplayAdapter } from './adapters/openaiConsumerQuota';
import { openaiApiDisplayAdapter } from './adapters/openaiApi';
import type {
  UsageCardSpec,
  UsageDisplayAdapter,
  UsageDisplayContext,
  UsageSnapshot,
} from './types';

const DISPLAY_ADAPTERS: UsageDisplayAdapter[] = [
  anthropicConsumerHybridDisplayAdapter,
  consumerStatusDisplayAdapter,
  openaiConsumerQuotaDisplayAdapter,
  openaiApiDisplayAdapter,
  anthropicApiDisplayAdapter,
  genericApiDisplayAdapter,
];

export type { UsageCardSpec, UsageDisplayContext, UsageRingSpec, UsageSnapshot } from './types';

export function resolveUsageCard(
  snapshot: UsageSnapshot,
  context: UsageDisplayContext,
): UsageCardSpec {
  const adapter = DISPLAY_ADAPTERS.find((candidate) => candidate.matches(snapshot))
    ?? genericApiDisplayAdapter;

  return adapter.toCard(snapshot, context);
}
