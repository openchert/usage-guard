import { buildProviderApiMetricCard } from './shared';
import type { UsageCardSpec, UsageDisplayAdapter } from '../types';

export const openaiApiDisplayAdapter: UsageDisplayAdapter = {
  id: 'openai-api',
  matches(snapshot) {
    return snapshot.provider === 'openai' && snapshot.source === 'api';
  },
  toCard(snapshot, context): UsageCardSpec {
    return buildProviderApiMetricCard('OpenAI', snapshot, context, {
      spendSource: 'organization costs',
      tokenSource: 'organization usage completions',
    });
  },
};
