<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import UsageRing from './UsageRing.svelte';
  import { providerMeta, providerRank } from './providerTheme';
  import { currentWindow, invoke, listen } from './tauri';
  import { resolveUsageCard, type UsageSnapshot } from './usageDisplay';

  const CARD_W = 110;
  const CARD_GAP = 6;
  const WIDGET_H = 90;
  const WIDGET_PAD = 8;
  const DEFAULT_REFRESH_INTERVAL_MS = 60_000;
  const REFRESH_EVENT = 'usageguard://refresh';

  let snapshots = [] as UsageSnapshot[];
  let isLoading = false;
  let refreshIntervalMs = DEFAULT_REFRESH_INTERVAL_MS;
  let refreshTimer: number | null = null;
  let lastRenderedCardCount: number | null = null;
  let unlistenRefresh: (() => void) | null = null;

  function cardSpec(snapshot: UsageSnapshot) {
    return resolveUsageCard(snapshot, {
      providerLabel: providerMeta(snapshot.provider).label,
    });
  }

  function displayLabel(snapshot: UsageSnapshot): string {
    return cardSpec(snapshot).displayLabel;
  }

  function sorted(items: UsageSnapshot[]): UsageSnapshot[] {
    return [...items].sort((a, b) => {
      return providerRank(a.provider) - providerRank(b.provider)
        || displayLabel(a).localeCompare(displayLabel(b));
    });
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

  function normalizeRefreshIntervalMs(value: number): number {
    if (!Number.isFinite(value) || value <= 0) return DEFAULT_REFRESH_INTERVAL_MS;
    return Math.round(value) * 1000;
  }

  function resetRefreshTimer(): void {
    if (refreshTimer !== null) window.clearInterval(refreshTimer);
    refreshTimer = window.setInterval(() => void requestRefresh(), refreshIntervalMs);
  }

  async function loadRefreshInterval(): Promise<void> {
    if (!invoke) return;

    try {
      const value = await invoke('get_refresh_interval_secs') as number;
      const nextRefreshIntervalMs = normalizeRefreshIntervalMs(value);
      if (nextRefreshIntervalMs !== refreshIntervalMs) {
        refreshIntervalMs = nextRefreshIntervalMs;
        resetRefreshTimer();
      }
    } catch (error) {
      console.error('get_refresh_interval_secs failed:', error);
    }
  }

  async function loadSnapshots(): Promise<void> {
    if (!invoke || isLoading) return;

    isLoading = true;
    try {
      const items = await invoke('get_snapshots') as UsageSnapshot[];
      snapshots = sorted(items);
      if (lastRenderedCardCount !== snapshots.length) {
        await resizeToFit(snapshots.length);
        lastRenderedCardCount = snapshots.length;
      }
    } catch (error) {
      console.error('get_snapshots failed:', error);
    } finally {
      isLoading = false;
    }
  }

  async function requestRefresh(): Promise<void> {
    if (!invoke) return;

    try {
      await invoke('refresh_snapshots');
    } catch (error) {
      console.error('refresh_snapshots failed:', error);
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
        void loadRefreshInterval();
        void loadSnapshots();
      });
    }

    await loadRefreshInterval();
    await loadSnapshots();
    resetRefreshTimer();
    void requestRefresh();
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
      {@const card = cardSpec(snapshot)}
      <div class="provider-card" class:provider-card-metrics={card.kind === 'metrics'} title={card.title}>
        <span class="card-name">{card.displayLabel}</span>
        {#if card.kind === 'quota'}
          <div class="rings">
            {#each card.rings as ring}
              <UsageRing
                ratio={ring.ratio}
                accent={provider.color}
                label={ring.label ?? ''}
                theme={provider.usageRing}
              />
            {/each}
          </div>
        {:else}
          <div class="metric-grid">
            {#each card.stats as stat}
              <div class="metric-tile">
                <span class="metric-value" style={`--metric-accent:${provider.color}`}>{stat.value}</span>
                <span class="metric-label">{stat.label}</span>
              </div>
            {/each}
          </div>
        {/if}
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
    background: var(--bg-surface);
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

  .provider-card-metrics {
    gap: 4px;
  }

  .metric-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
    flex: 1;
    align-items: center;
  }

  .metric-tile {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 3px;
    min-width: 0;
  }

  .metric-label {
    font-size: 9px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: rgba(155, 162, 182, 0.7);
  }

  .metric-value {
    font-size: 13px;
    font-weight: 700;
    line-height: 1;
    color: var(--metric-accent);
  }
</style>
