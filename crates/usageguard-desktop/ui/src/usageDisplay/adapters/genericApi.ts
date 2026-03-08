import { buildGenericApiCard } from './shared';
import type { UsageCardSpec, UsageDisplayAdapter } from '../types';

export const genericApiDisplayAdapter: UsageDisplayAdapter = {
  id: 'generic-api',
  matches() {
    return true;
  },
  toCard(snapshot, context): UsageCardSpec {
    return buildGenericApiCard(snapshot, context);
  },
};
