export interface ApiMetricWindow {
  spend_usd: number;
  tokens_in: number;
  tokens_out: number;
  requests?: number | null;
}

export interface ApiMetricCard {
  today: ApiMetricWindow;
  rolling_30d: ApiMetricWindow;
}

export interface ConsumerQuotaWindow {
  available: boolean;
  used_percent?: number | null;
  reset_at?: string | null;
}

export interface ConsumerQuotaCard {
  primary?: ConsumerQuotaWindow | null;
  secondary?: ConsumerQuotaWindow | null;
}

export interface Alert {
  level: string;
  code: string;
  message: string;
}

export interface UsageSnapshot {
  provider: string;
  account_label: string;
  spent_usd: number | null;
  limit_usd: number | null;
  tokens_in: number | null;
  tokens_out: number | null;
  inactive_hours: number | null;
  source: string;
  status_code?: string | null;
  status_message?: string | null;
  api_metrics?: ApiMetricCard | null;
  consumer_quota?: ConsumerQuotaCard | null;
  primary_reset_at?: string | null;
  secondary_reset_at?: string | null;
  alerts?: Alert[] | null;
}

export interface UsageRingSpec {
  label?: string;
  ratio: number;
}

interface UsageCardBase {
  displayLabel: string;
  title: string;
}

export interface QuotaUsageCardSpec extends UsageCardBase {
  kind: 'quota';
  rings: UsageRingSpec[];
}

export interface MetricStatSpec {
  label: string;
  value: string;
  detail?: string;
}

export interface MetricsUsageCardSpec extends UsageCardBase {
  kind: 'metrics';
  stats: MetricStatSpec[];
}

export interface HybridUsageCardSpec extends UsageCardBase {
  kind: 'hybrid';
  rings: UsageRingSpec[];
  stats: MetricStatSpec[];
}

export interface StatusUsageCardSpec extends UsageCardBase {
  kind: 'status';
  /** Visual state of the ring animation. */
  statusKind: 'loading' | 'waiting' | 'error';
  /** Short label shown beneath the ring (e.g. "syncing", "waiting"). */
  label: string;
}

export type UsageCardSpec = QuotaUsageCardSpec | MetricsUsageCardSpec | HybridUsageCardSpec | StatusUsageCardSpec;

export interface UsageDisplayContext {
  providerLabel: string;
}

export interface UsageDisplayAdapter {
  id: string;
  matches(snapshot: UsageSnapshot): boolean;
  toCard(snapshot: UsageSnapshot, context: UsageDisplayContext): UsageCardSpec;
}
