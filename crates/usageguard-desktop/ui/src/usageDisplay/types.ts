export interface UsageSnapshot {
  provider: string;
  account_label: string;
  spent_usd: number | null;
  limit_usd: number | null;
  tokens_in: number | null;
  tokens_out: number | null;
  inactive_hours: number | null;
  source: string;
}

export interface UsageRingSpec {
  label?: string;
  ratio: number;
}

export interface UsageCardSpec {
  displayLabel: string;
  title: string;
  rings: UsageRingSpec[];
}

export interface UsageDisplayContext {
  providerLabel: string;
}

export interface UsageDisplayAdapter {
  id: string;
  matches(snapshot: UsageSnapshot): boolean;
  toCard(snapshot: UsageSnapshot, context: UsageDisplayContext): UsageCardSpec;
}
