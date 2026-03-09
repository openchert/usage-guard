<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import UsageRing from './UsageRing.svelte';
  import { currentWindow, invoke, listen } from './tauri';
  import { providerMeta, providerRank } from './providerTheme';
  import { resolveUsageCard } from './usageDisplay';

  interface UsageSnapshot {
    provider: string;
    account_label: string;
    spent_usd: number | null;
    limit_usd: number | null;
    tokens_in: number | null;
    tokens_out: number | null;
    inactive_hours: number | null;
    source: string; // "api" | "oauth" | "oauth-error:*" | "demo" | …
  }

  const CARD_W = 110;
  const CARD_GAP = 6;
  const WIDGET_H = 90;
  const WIDGET_PAD = 8;
  const REFRESH_MS = 30_000;
  const REFRESH_EVENT = 'usageguard://refresh';

  let snapshots = [] as UsageSnapshot[];
  let isLoading = false;
  let refreshTimer: number | null = null;
  let unlistenRefresh: (() => void) | null = null;

  function cardSpec(snapshot: UsageSnapshot) {
    return resolveUsageCard(snapshot, {
      providerLabel: providerMeta(snapshot.provider).label,
    });
  }

  function displayLabel(snapshot: UsageSnapshot): string {
    return cardSpec(snapshot).displayLabel;
  }

  function cardTitle(snapshot: UsageSnapshot): string {
    return cardSpec(snapshot).title;
  }

  function sorted(items: UsageSnapshot[]): UsageSnapshot[] {
    return [...items].sort((a, b) => {
      return providerRank(a.provider) - providerRank(b.provider)
        || displayLabel(a).localeCompare(displayLabel(b));
    });
  }

  function oauthRemainingRatio(percent: number | null): number {
    if (percent == null || !Number.isFinite(percent)) return 0;
    return Math.min(1, Math.max(0, 1 - percent / 100));
  }

  function weekRatio(snapshot: UsageSnapshot): number {
    // OAuth: spent_usd holds secondary_window used_percent (0–100).
    // Show REMAINING capacity so the ring drains as the limit is consumed.
    if (snapshot.source === 'oauth') {
      return oauthRemainingRatio(snapshot.spent_usd);
    }
    if (snapshot.limit_usd && snapshot.limit_usd > 0 && snapshot.spent_usd != null) {
      return Math.min(1, snapshot.spent_usd / snapshot.limit_usd);
    }
    const total = (snapshot.tokens_in ?? 0) + (snapshot.tokens_out ?? 0);
    if (total > 0) return Math.min(1, total / 1_200_000);
    return 0;
  }

  function fiveHourRatio(snapshot: UsageSnapshot): number {
    // OAuth: tokens_in holds primary_window used_percent (0–100).
    // Show REMAINING capacity so the ring drains as the limit is consumed.
    if (snapshot.source === 'oauth') {
      return oauthRemainingRatio(snapshot.tokens_in);
    }
    const weekly = weekRatio(snapshot);
    const inactive = snapshot.inactive_hours ?? 0;
    const activeFactor = Math.max(0.2, 1 - Math.min(inactive, 24) / 24);
    return Math.min(1, weekly * (0.22 + activeFactor * 0.35));
  }

  async function resizeToFit(count: number): Promise<void> {
    if (!invoke || !currentWindow) return;

    const visibleCards = Math.max(count, 1);
    const scale = await currentWindow.scaleFactor();
    const size = await currentWindow.innerSize();
    const position = await currentWindow.outerPosition();

    const physRight = position.x + size.width;
    const physBottom = position.y + size.height;
    const nextWidth = Math.round((2 * WIDGET_PAD + visibleCards * CARD_W + (visibleCards - 1) * CARD_GAP) * scale);
    const nextHeight = Math.round(WIDGET_H * scale);

    await invoke('set_window_rect', {
      x: physRight - nextWidth,
      y: physBottom - nextHeight,
      w: nextWidth,
      h: nextHeight,
    });
  }

  async function loadSnapshots(): Promise<void> {
    if (!invoke || isLoading) return;

    isLoading = true;
    try {
      const items = await invoke('get_snapshots') as UsageSnapshot[];
      snapshots = sorted(items);
      await resizeToFit(snapshots.length);
    } catch (error) {
      console.error('get_snapshots failed:', error);
    } finally {
      isLoading = false;
    }
  }

  function onContextMenu(event: MouseEvent): void {
    event.preventDefault();
    if (!invoke) return;

    void invoke('show_context_menu', { x: event.clientX, y: event.clientY }).catch((error) => {
      console.error('show_context_menu failed:', error);
    });
  }

  function onSelectStart(event: Event): void {
    const target = event.target as HTMLElement | null;
    if (target?.closest('input, textarea, button, select')) return;
    event.preventDefault();
  }

  async function startDrag(event: MouseEvent): Promise<void> {
    if (!currentWindow || event.button !== 0) return;
    try {
      await currentWindow.startDragging();
    } catch {
      // Ignore drag failures when the OS rejects the gesture.
    }
  }

  onMount(async () => {
    document.addEventListener('contextmenu', onContextMenu);
    document.addEventListener('selectstart', onSelectStart);

    if (listen) {
      unlistenRefresh = await listen(REFRESH_EVENT, () => {
        void loadSnapshots();
      });
    }

    await loadSnapshots();
    refreshTimer = window.setInterval(() => void loadSnapshots(), REFRESH_MS);
  });

  onDestroy(() => {
    document.removeEventListener('contextmenu', onContextMenu);
    document.removeEventListener('selectstart', onSelectStart);
    unlistenRefresh?.();
    if (refreshTimer !== null) window.clearInterval(refreshTimer);
  });
</script>

<div class="widget-shell" on:mousedown={startDrag} role="presentation">
  {#if snapshots.length === 0 && !isLoading}
    <div class="empty-state">
      <span class="empty-copy">Right-click to connect a provider</span>
    </div>
  {:else}
    {#each snapshots as snapshot}
      {@const provider = providerMeta(snapshot.provider)}
      <div class="provider-card" title={cardTitle(snapshot)}>
        <span class="card-name">{displayLabel(snapshot)}</span>
        <div class="rings">
          <UsageRing ratio={fiveHourRatio(snapshot)} accent={provider.color} label="5h" theme={provider.usageRing} />
          <UsageRing ratio={weekRatio(snapshot)} accent={provider.color} theme={provider.usageRing} />
        </div>
      </div>
    {/each}
  {/if}
</div>

<style>
  .widget-shell {
    position: absolute;
    inset: 0;
    display: flex;
    gap: 6px;
    height: 100%;
    padding: 8px;
    border: 1px solid rgba(255, 255, 255, 0.07);
    border-radius: 12px;
    background: #1e1f2e;
    cursor: grab;
    user-select: none;
    -webkit-user-select: none;
  }

  .widget-shell:active {
    cursor: grabbing;
  }

  .empty-state {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    padding: 0 8px;
  }

  .empty-copy {
    font-size: 11px;
    color: rgba(133, 139, 160, 0.9);
  }

  .provider-card {
    flex: 0 0 110px;
    padding: 9px 8px 8px;
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 8px;
    background: rgba(255, 255, 255, 0.03);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .card-name {
    font-size: 11px;
    font-weight: 600;
    line-height: 1.1;
    color: rgba(229, 232, 242, 0.96);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .rings {
    display: flex;
    justify-content: center;
    align-items: center;
    gap: 12px;
    flex: 1;
  }
</style>
