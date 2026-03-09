import { anthropicApiDisplayAdapter } from './adapters/anthropicApi';
import { anthropicOauthDisplayAdapter } from './adapters/anthropicOauth';
import { genericApiDisplayAdapter } from './adapters/genericApi';
import { openaiOauthDisplayAdapter } from './adapters/openaiOauth';
import { openaiApiDisplayAdapter } from './adapters/openaiApi';
import type {
  UsageCardSpec,
  UsageDisplayAdapter,
  UsageDisplayContext,
  UsageSnapshot,
} from './types';

const DISPLAY_ADAPTERS: UsageDisplayAdapter[] = [
  openaiOauthDisplayAdapter,
  anthropicOauthDisplayAdapter,
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
