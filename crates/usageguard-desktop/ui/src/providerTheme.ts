import type { UsageRingTheme } from './usageRingTheme';

export interface ProviderTheme {
  label: string;
  color: string;
  usageRing?: UsageRingTheme;
}

export const PROVIDER_META: Record<string, ProviderTheme> = {
  openai: { label: 'OpenAI', color: '#1fa97c' },
  anthropic: { label: 'Anthropic', color: '#d97a4e' },
  gemini: { label: 'Gemini', color: '#4a78e0' },
  groq: { label: 'Groq', color: '#7060e8' },
  mistral: { label: 'Mistral', color: '#e87c2a' },
  copilot: { label: 'Copilot', color: '#4f8cff' },
  cursor: { label: 'Cursor', color: '#f08a59' },
};

export const PROVIDER_ORDER = [
  'openai', 'anthropic', 'gemini', 'groq', 'mistral', 'copilot', 'cursor',
];

export function providerMeta(provider: string): ProviderTheme {
  return PROVIDER_META[provider] ?? { label: provider, color: '#5a6680' };
}

export function providerRank(provider: string) {
  const index = PROVIDER_ORDER.indexOf(provider);
  return index === -1 ? 999 : index;
}
