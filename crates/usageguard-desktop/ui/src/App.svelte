<script lang="ts">
  import { onDestroy, onMount } from 'svelte';

  interface UsageSnapshot {
    provider: string;
    account_label: string;
    spent_usd: number | null;
    limit_usd: number | null;
    tokens_in: number | null;
    tokens_out: number | null;
    inactive_hours: number | null;
    source: string;
  }

  const PROVIDER_META: Record<string, { label: string; color: string }> = {
    openai: { label: 'OpenAI', color: '#1fa97c' },
    anthropic: { label: 'Claude', color: '#d97a4e' },
    gemini: { label: 'Gemini', color: '#4a78e0' },
    groq: { label: 'Groq', color: '#7060e8' },
    openrouter: { label: 'OpenRouter', color: '#3aa8b8' },
    mistral: { label: 'Mistral', color: '#e87c2a' },
    together: { label: 'Together', color: '#4f96ff' },
    azure_openai: { label: 'Azure', color: '#0078d4' },
    deepseek: { label: 'DeepSeek', color: '#4facfe' },
  };

  const PROVIDER_ORDER = [
    'openai', 'anthropic', 'gemini', 'groq', 'openrouter',
    'mistral', 'together', 'azure_openai', 'deepseek',
  ];

  const CARD_W = 110;
  const CARD_GAP = 6;
  const WIDGET_H = 100;
  const WIDGET_PAD = 8;
  const REFRESH_MS = 30_000;
  const REFRESH_EVENT = 'usageguard://refresh';

  const tauri = (window as any).__TAURI__;
  const invoke = tauri?.core?.invoke as ((cmd: string, args?: Record<string, unknown>) => Promise<any>) | undefined;
  const listen = tauri?.event?.listen as
    | ((event: string, handler: (event: unknown) => void) => Promise<() => void>)
    | undefined;
  const win = tauri?.window?.getCurrentWindow?.() ?? tauri?.window?.getCurrent?.() ?? null;

  let snapshots = [] as UsageSnapshot[];
  let isLoading = false;
  let refreshTimer: number | null = null;
  let unlistenRefresh: (() => void) | null = null;

  function meta(provider: string) {
    return PROVIDER_META[provider] ?? { label: provider, color: '#5a6680' };
  }

  function sorted(items: UsageSnapshot[]): UsageSnapshot[] {
    return [...items].sort((a, b) => {
      const rank = (id: string) => {
        const index = PROVIDER_ORDER.indexOf(id);
        return index === -1 ? 999 : index;
      };

      return rank(a.provider) - rank(b.provider);
    });
  }

  function weekRatio(snapshot: UsageSnapshot): number {
    if (snapshot.limit_usd && snapshot.limit_usd > 0 && snapshot.spent_usd != null) {
      return Math.min(1, snapshot.spent_usd / snapshot.limit_usd);
    }

    const total = (snapshot.tokens_in ?? 0) + (snapshot.tokens_out ?? 0);
    if (total > 0) return Math.min(1, total / 1_200_000);
    return 0;
  }

  function fiveHourRatio(snapshot: UsageSnapshot): number {
    const weekly = weekRatio(snapshot);
    const inactive = snapshot.inactive_hours ?? 0;
    const activeFactor = Math.max(0.2, 1 - Math.min(inactive, 24) / 24);
    return Math.min(1, weekly * (0.22 + activeFactor * 0.35));
  }

  function displayLabel(snapshot: UsageSnapshot): string {
    if (snapshot.provider === 'anthropic') return 'Claude';
    if (snapshot.account_label?.trim()) return snapshot.account_label;
    return meta(snapshot.provider).label;
  }

  async function resizeToFit(count: number): Promise<void> {
    if (!invoke || !win) return;

    const visibleCards = Math.max(count, 1);
    const scale = await win.scaleFactor();
    const size = await win.innerSize();
    const position = await win.outerPosition();

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
    if (target?.closest('input, textarea')) return;
    event.preventDefault();
  }

  async function startDrag(event: MouseEvent): Promise<void> {
    if (!win || event.button !== 0) return;
    try {
      await win.startDragging();
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

<div class="widget" on:mousedown={startDrag} role="presentation">
  {#if snapshots.length === 0}
    <div class="empty">Loading...</div>
  {:else}
    {#each snapshots as snapshot}
      {@const provider = meta(snapshot.provider)}
      <div class="provider-card">
        <span class="card-name">{displayLabel(snapshot)}</span>
        <div class="rings">
          <div class="ring-item">
            <div class="ring" style="--r:{fiveHourRatio(snapshot).toFixed(4)};--c:{provider.color}"></div>
            <span class="ring-label">5h</span>
          </div>
          <div class="ring-item">
            <div class="ring" style="--r:{weekRatio(snapshot).toFixed(4)};--c:{provider.color}"></div>
            <span class="ring-label">week</span>
          </div>
        </div>
      </div>
    {/each}
  {/if}
</div>
