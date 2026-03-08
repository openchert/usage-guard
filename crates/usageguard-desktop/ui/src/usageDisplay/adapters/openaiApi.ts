import { buildGenericApiCard, buildProviderApiTitle } from './shared';
import type { UsageCardSpec, UsageDisplayAdapter } from '../types';

export const openaiApiDisplayAdapter: UsageDisplayAdapter = {
  id: 'openai-api',
  matches(snapshot) {
    return snapshot.provider === 'openai' && snapshot.source === 'api';
  },
  toCard(snapshot, context): UsageCardSpec {
    const card = buildGenericApiCard(snapshot, context);
    return {
      ...card,
      title: buildProviderApiTitle('OpenAI', snapshot, context),
    };
  },
};
