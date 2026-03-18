import { buildCardTitle, formatResetTime, quotaResetAt } from './shared';
import type {
  UsageCardSpec,
  UsageDisplayAdapter,
  UsageDisplayContext,
  UsageSnapshot,
} from '../types';

type StatusKind = 'loading' | 'waiting' | 'auth' | 'error';

function statusKindForCode(code: string | null | undefined): StatusKind {
  if (code === 'consumer_local_usage_pending') return 'loading';
  if (code === 'consumer_local_waiting_for_usage') return 'waiting';
  if (code === 'oauth_reauth_required') return 'auth';
  return 'error';
}

function statusLabel(kind: StatusKind): string {
  if (kind === 'loading') return 'syncing';
  if (kind === 'waiting') return 'start session';
  if (kind === 'auth') return 'sign in';
  return 'unavailable';
}

function statusTitleLines(
  snapshot: UsageSnapshot,
  displayLabel: string,
  kind: StatusKind,
): string[] {
  const isOpenAI = snapshot.provider === 'openai';
  const lines = [displayLabel];

  if (kind === 'loading') {
    lines.push(isOpenAI ? 'Fetching Codex quota…' : 'Fetching Claude Code quota…');
    lines.push('Refreshes every 5 minutes');
    const primaryReset = formatResetTime(quotaResetAt(snapshot, 'primary'));
    if (primaryReset) lines.push(`5h resets: ${primaryReset}`);
  } else if (kind === 'waiting') {
    lines.push('No session data yet');
    lines.push(
      isOpenAI
        ? 'Run a Codex task to begin tracking'
        : 'Run a Claude Code task to begin tracking',
    );
  } else if (kind === 'auth') {
    lines.push('Sign-in expired');
    lines.push('Open settings to re-authenticate');
  } else {
    lines.push('Usage data unavailable');
    if (snapshot.status_message) lines.push(snapshot.status_message);
    lines.push('Check connection or try refreshing');
  }

  return lines;
}

function displayLabel(snapshot: UsageSnapshot, context: UsageDisplayContext): string {
  return snapshot.account_label?.trim() || context.providerLabel;
}

export const consumerStatusDisplayAdapter: UsageDisplayAdapter = {
  id: 'consumer-status',
  matches(snapshot) {
    return snapshot.source === 'consumer_local_status';
  },
  toCard(snapshot, context): UsageCardSpec {
    const label = displayLabel(snapshot, context);
    const kind = statusKindForCode(snapshot.status_code);
    const titleLines = statusTitleLines(snapshot, label, kind);

    return {
      kind: 'status',
      displayLabel: label,
      title: buildCardTitle(snapshot, titleLines),
      statusKind: kind,
      label: statusLabel(kind),
    };
  },
};
