import type { UsageRingTheme } from './usageRingTheme';

export interface ProviderTheme {
  label: string;
  color: string;
  usageRing?: UsageRingTheme;
}

export const PROVIDER_META: Record<string, ProviderTheme> = {
  openai: { label: 'OpenAI', color: '#1fa97c' },
  anthropic: { label: 'Anthropic', color: '#d97a4e' },
  cursor: { label: 'Cursor', color: '#f08a59' },
};

export const PROVIDER_ORDER = ['openai', 'anthropic', 'cursor'];

export function providerMeta(provider: string): ProviderTheme {
  return PROVIDER_META[provider] ?? { label: provider, color: '#5a6680' };
}

export function providerRank(provider: string) {
  const index = PROVIDER_ORDER.indexOf(provider);
  return index === -1 ? 999 : index;
}
