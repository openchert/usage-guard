import { buildGenericApiCard, buildProviderApiTitle } from './shared';
import type { UsageCardSpec, UsageDisplayAdapter } from '../types';

export const anthropicApiDisplayAdapter: UsageDisplayAdapter = {
  id: 'anthropic-api',
  matches(snapshot) {
    return snapshot.provider === 'anthropic' && snapshot.source === 'api';
  },
  toCard(snapshot, context): UsageCardSpec {
    const card = buildGenericApiCard(snapshot, context);
    return {
      ...card,
      title: buildProviderApiTitle('Anthropic', snapshot, context),
    };
  },
};
