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

  interface OAuthStatus {
    connected: boolean;
  }

  interface WidgetConfig {
    light_mode: boolean;
    provider_accounts?: Array<unknown>;
  }

  let snapshots = [] as UsageSnapshot[];
  let isLoading = false;
  let bootstrapComplete = false;
  let hasConfiguredSources = false;
  let initialShown = false;
  let refreshIntervalMs = DEFAULT_REFRESH_INTERVAL_MS;
  let refreshTimer: number | null = null;
  let startupRevealTimer: number | null = null;
  let lastRenderedCardCount: number | null = null;
  let unlistenRefresh: (() => void) | null = null;

  type AlertLevel = 'critical' | 'warning' | 'info';

  function cardSpec(snapshot: UsageSnapshot) {
    return resolveUsageCard(snapshot, {
      providerLabel: providerMeta(snapshot.provider).label,
    });
  }

  function displayLabel(snapshot: UsageSnapshot): string {
    return cardSpec(snapshot).displayLabel;
  }

  function cardAlertLevel(snapshot: UsageSnapshot): AlertLevel | null {
    const alerts = snapshot.alerts ?? [];
    if (alerts.some((alert) => alert.level === 'critical')) return 'critical';
    if (alerts.some((alert) => alert.level === 'warning')) return 'warning';
    if (alerts.some((alert) => alert.level === 'info')) return 'info';
    return null;
  }

  function alertBadgeLabel(level: AlertLevel): string {
    return level === 'info' ? 'i' : '!';
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

  async function applyTheme(): Promise<void> {
    if (!invoke) return;
    try {
      const cfg = await invoke('get_config') as WidgetConfig;
      document.documentElement.classList.toggle('light-mode', cfg.light_mode);
    } catch (error) {
      console.error('get_config (theme) failed:', error);
    }
  }

  async function loadConfiguredSources(): Promise<void> {
    if (!invoke) return;

    try {
      const [cfg, openaiStatus, anthropicStatus] = await Promise.all([
        invoke('get_config') as Promise<WidgetConfig>,
        invoke('get_openai_oauth_status') as Promise<OAuthStatus>,
        invoke('get_anthropic_oauth_status') as Promise<OAuthStatus>,
      ]);
      hasConfiguredSources =
        (cfg.provider_accounts?.length ?? 0) > 0
        || openaiStatus.connected
        || anthropicStatus.connected;
    } catch (error) {
      console.error('widget startup state check failed:', error);
      hasConfiguredSources = false;
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

  async function showWindowOnce(): Promise<void> {
    if (initialShown || !currentWindow) return;
    initialShown = true;
    try {
      await currentWindow.show();
    } catch (error) {
      console.error('show window failed:', error);
    }
  }

  async function revealWhenReady(): Promise<void> {
    if (bootstrapComplete) return;
    if (hasConfiguredSources && snapshots.length === 0) return;

    bootstrapComplete = true;
    await showWindowOnce();
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
      unlistenRefresh = await listen(REFRESH_EVENT, async () => {
        await applyTheme();
        await loadRefreshInterval();
        await loadSnapshots();
        await revealWhenReady();
      });
    }

    await applyTheme();
    await loadRefreshInterval();
    await loadConfiguredSources();

    if (!hasConfiguredSources) {
      await loadSnapshots();
      await revealWhenReady();
    }

    resetRefreshTimer();
    void requestRefresh();

    // Fallback: keep the widget hidden until startup state is known.
    startupRevealTimer = window.setTimeout(() => {
      void (async () => {
        await loadSnapshots();
        await revealWhenReady();
      })();
    }, 8000);
  });

  onDestroy(() => {
    document.removeEventListener('contextmenu', onContextMenu);
    document.removeEventListener('selectstart', onSelectStart);
    unlistenRefresh?.();
    if (refreshTimer !== null) window.clearInterval(refreshTimer);
    if (startupRevealTimer !== null) window.clearTimeout(startupRevealTimer);
  });
</script>

<div class="widget-shell" on:mousedown={startDrag} role="presentation">
  {#if bootstrapComplete && snapshots.length === 0 && !isLoading && !hasConfiguredSources}
    <div class="empty-state">
      <span class="empty-copy">Right-click to connect a provider</span>
    </div>
  {:else}
    {#each snapshots as snapshot}
      {@const provider = providerMeta(snapshot.provider)}
      {@const card = cardSpec(snapshot)}
      {@const alertLevel = cardAlertLevel(snapshot)}
      <div
        class="provider-card"
        class:provider-card-metrics={card.kind === 'metrics'}
        data-alert-level={alertLevel ?? undefined}
        title={card.title}
      >
        {#if alertLevel}
          <span class="alert-badge" aria-hidden="true">{alertBadgeLabel(alertLevel)}</span>
        {/if}
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
    border: 1px solid var(--border-panel);
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
    color: var(--text-muted);
  }

  .provider-card {
    position: relative;
    flex: 0 0 110px;
    padding: 9px 8px 8px;
    border: 1px solid var(--border-card);
    border-radius: 8px;
    background: var(--surface-card);
    display: flex;
    flex-direction: column;
    gap: 8px;
    transition: border-color 120ms ease, box-shadow 120ms ease;
  }

  .provider-card[data-alert-level='critical'] {
    border-color: rgba(214, 84, 84, 0.95);
    box-shadow: inset 0 0 0 1px rgba(214, 84, 84, 0.22);
  }

  .provider-card[data-alert-level='warning'] {
    border-color: rgba(224, 170, 66, 0.95);
    box-shadow: inset 0 0 0 1px rgba(224, 170, 66, 0.2);
  }

  .provider-card[data-alert-level='info'] {
    border-color: rgba(94, 156, 255, 0.95);
    box-shadow: inset 0 0 0 1px rgba(94, 156, 255, 0.18);
  }

  .alert-badge {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 14px;
    height: 14px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 999px;
    font-size: 9px;
    font-weight: 700;
    line-height: 1;
    color: #fff;
    background: rgba(94, 156, 255, 0.92);
    border: 1px solid rgba(255, 255, 255, 0.18);
  }

  .provider-card[data-alert-level='critical'] .alert-badge {
    background: rgba(214, 84, 84, 0.95);
  }

  .provider-card[data-alert-level='warning'] .alert-badge {
    background: rgba(224, 170, 66, 0.95);
    color: rgba(36, 22, 0, 0.92);
  }

  .card-name {
    font-size: 11px;
    font-weight: 600;
    line-height: 1.1;
    color: var(--text-hi);
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
    color: var(--text-lo);
  }

  .metric-value {
    font-size: 13px;
    font-weight: 700;
    line-height: 1;
    color: var(--metric-accent);
  }
</style>
