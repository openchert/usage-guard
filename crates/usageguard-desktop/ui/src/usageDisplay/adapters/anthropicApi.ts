import { buildProviderApiMetricCard } from './shared';
import type { UsageCardSpec, UsageDisplayAdapter } from '../types';

export const anthropicApiDisplayAdapter: UsageDisplayAdapter = {
  id: 'anthropic-api',
  matches(snapshot) {
    return snapshot.provider === 'anthropic' && snapshot.source === 'api';
  },
  toCard(snapshot, context): UsageCardSpec {
    return buildProviderApiMetricCard('Anthropic', snapshot, context, {
      spendSource: 'cost report',
      tokenSource: 'messages usage report',
    });
  },
};
