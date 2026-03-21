<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { currentWindow, invoke, listen } from './tauri';

  interface ConsumerStatus {
    connected: boolean;
    plan_type: string | null;
    label: string | null;
    alerts_5h_enabled: boolean;
    alerts_week_enabled: boolean;
    supports_usage: boolean;
    supports_5h_usage: boolean;
    supports_week_usage: boolean;
    source_label: string;
    status_message: string | null;
  }

  let isLoading = true;
  let errorMessage = '';
  let openaiConsumerStatus: ConsumerStatus = {
    connected: false,
    plan_type: null,
    label: null,
    alerts_5h_enabled: true,
    alerts_week_enabled: true,
    supports_usage: false,
    supports_5h_usage: false,
    supports_week_usage: false,
    source_label: 'Codex local client',
    status_message: null,
  };
  let anthropicConsumerStatus: ConsumerStatus = {
    connected: false,
    plan_type: null,
    label: null,
    alerts_5h_enabled: true,
    alerts_week_enabled: true,
    supports_usage: false,
    supports_5h_usage: false,
    supports_week_usage: false,
    source_label: 'Claude Code local client',
    status_message: null,
  };
  let openaiEditLabel = '';
  let anthropicEditLabel = '';
  let openaiLabelFocused = false;
  let anthropicLabelFocused = false;
  let unlistenRefresh: (() => void) | null = null;

  const REFRESH_EVENT = 'usageguard://refresh';

  async function applyTheme(): Promise<void> {
    if (!invoke) return;
    try {
      const cfg = await invoke('get_config') as { light_mode: boolean };
      document.documentElement.classList.toggle('light-mode', cfg.light_mode);
    } catch {
      // Ignore theme refresh failures.
    }
  }

  function defaultOpenAILabel(status: ConsumerStatus): string {
    return status.label || (status.plan_type ? `Codex ${status.plan_type}` : 'Codex');
  }

  function defaultAnthropicLabel(status: ConsumerStatus): string {
    return status.label || (status.plan_type ? `Claude Code ${status.plan_type}` : 'Claude Code');
  }

  async function closeWindow(): Promise<void> {
    if (!currentWindow) return;
    try {
      await currentWindow.close();
    } catch {
      // Ignore close failures.
    }
  }

  async function startDrag(event: MouseEvent): Promise<void> {
    if (!currentWindow || event.button !== 0) return;
    try {
      await currentWindow.startDragging();
    } catch {
      // Ignore drag failures when the OS rejects the gesture.
    }
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    void closeWindow();
  }

  async function loadSettings(showLoading = false): Promise<void> {
    if (!invoke) return;
    if (showLoading) {
      isLoading = true;
    }
    try {
      const [openaiStatus, anthropicStatus] = await Promise.all([
        invoke('get_openai_consumer_status') as Promise<ConsumerStatus>,
        invoke('get_anthropic_consumer_status') as Promise<ConsumerStatus>,
      ]);
      openaiConsumerStatus = openaiStatus;
      anthropicConsumerStatus = anthropicStatus;
      if (!openaiLabelFocused) {
        openaiEditLabel = defaultOpenAILabel(openaiStatus);
      }
      if (!anthropicLabelFocused) {
        anthropicEditLabel = defaultAnthropicLabel(anthropicStatus);
      }
    } catch (error) {
      errorMessage = String(error);
    } finally {
      if (showLoading) {
        isLoading = false;
      }
    }
  }

  async function saveConsumerLabel(provider: 'openai' | 'anthropic', label: string): Promise<void> {
    if (!invoke) return;
    errorMessage = '';
    try {
      await invoke('set_consumer_label', { provider, label });
    } catch (error) {
      errorMessage = String(error);
    }
  }

  async function setConsumerWindowAlertsEnabled(
    provider: 'openai' | 'anthropic',
    windowKey: '5h' | 'week',
    enabled: boolean,
  ): Promise<void> {
    if (!invoke) return;
    const target = provider === 'openai' ? openaiConsumerStatus : anthropicConsumerStatus;
    const field = windowKey === '5h' ? 'alerts_5h_enabled' : 'alerts_week_enabled';
    const previous = target[field];

    if (provider === 'openai') {
      openaiConsumerStatus = { ...openaiConsumerStatus, [field]: enabled };
    } else {
      anthropicConsumerStatus = { ...anthropicConsumerStatus, [field]: enabled };
    }

    errorMessage = '';
    try {
      await invoke('set_consumer_window_alerts_enabled', { provider, windowKey, enabled });
    } catch (error) {
      if (provider === 'openai') {
        openaiConsumerStatus = { ...openaiConsumerStatus, [field]: previous };
      } else {
        anthropicConsumerStatus = { ...anthropicConsumerStatus, [field]: previous };
      }
      errorMessage = String(error);
    }
  }

  onMount(async () => {
    window.addEventListener('keydown', onKeydown);
    await applyTheme();
    await loadSettings(true);
    if (listen) {
      unlistenRefresh = await listen(REFRESH_EVENT, async () => {
        await applyTheme();
        await loadSettings();
      });
    }
  });

  onDestroy(() => {
    window.removeEventListener('keydown', onKeydown);
    unlistenRefresh?.();
  });
</script>

<div class="shell" on:contextmenu|preventDefault role="presentation">
  <div class="panel">
    <header class="bar" on:mousedown={startDrag} role="presentation">
      <span class="bar-title">Connections</span>
      <div class="bar-spacer"></div>
      <button class="bar-btn bar-btn-close" type="button" title="Close" on:mousedown|stopPropagation on:click|stopPropagation={closeWindow}>x</button>
    </header>

    <div class="body">
      {#if isLoading}
        <div class="placeholder">Checking local connections...</div>
      {/if}

      <div class="consumer-row" class:consumer-connected={openaiConsumerStatus.connected}>
        <div class="account-dot" style="--accent:{openaiConsumerStatus.connected ? '#10a37f' : 'rgba(130, 138, 165, 0.35)'}"></div>
        {#if openaiConsumerStatus.connected}
          <input
            class="consumer-name"
            type="text"
            bind:value={openaiEditLabel}
            on:focus={() => { openaiLabelFocused = true; }}
            on:blur={() => {
              openaiLabelFocused = false;
              void saveConsumerLabel('openai', openaiEditLabel);
            }}
            on:mousedown|stopPropagation
          />
        {:else}
          <span class="consumer-name static-label">{defaultOpenAILabel(openaiConsumerStatus)}</span>
        {/if}
        <span class="consumer-provider-label">{openaiConsumerStatus.source_label}</span>
      </div>
      <div class="consumer-subrow">
        <span class="consumer-subrow-label">Alerts</span>
        <label class="consumer-checkbox">
          <input
            type="checkbox"
            checked={openaiConsumerStatus.alerts_5h_enabled}
            disabled={!openaiConsumerStatus.supports_5h_usage}
            on:change={(event) => void setConsumerWindowAlertsEnabled('openai', '5h', (event.currentTarget as HTMLInputElement).checked)}
            on:mousedown|stopPropagation
          />
          <span>5h</span>
        </label>
        <label class="consumer-checkbox">
          <input
            type="checkbox"
            checked={openaiConsumerStatus.alerts_week_enabled}
            disabled={!openaiConsumerStatus.supports_week_usage}
            on:change={(event) => void setConsumerWindowAlertsEnabled('openai', 'week', (event.currentTarget as HTMLInputElement).checked)}
            on:mousedown|stopPropagation
          />
          <span>Week</span>
        </label>
      </div>
      {#if openaiConsumerStatus.status_message}
        <span class="field-help">{openaiConsumerStatus.status_message}</span>
      {/if}

      <div class="consumer-row" class:consumer-connected={anthropicConsumerStatus.connected}>
        <div class="account-dot" style="--accent:{anthropicConsumerStatus.connected ? '#d97a4e' : 'rgba(130, 138, 165, 0.35)'}"></div>
        {#if anthropicConsumerStatus.connected}
          <input
            class="consumer-name"
            type="text"
            bind:value={anthropicEditLabel}
            on:focus={() => { anthropicLabelFocused = true; }}
            on:blur={() => {
              anthropicLabelFocused = false;
              void saveConsumerLabel('anthropic', anthropicEditLabel);
            }}
            on:mousedown|stopPropagation
          />
        {:else}
          <span class="consumer-name static-label">{defaultAnthropicLabel(anthropicConsumerStatus)}</span>
        {/if}
        <span class="consumer-provider-label">{anthropicConsumerStatus.source_label}</span>
      </div>
      <div class="consumer-subrow">
        <span class="consumer-subrow-label">Alerts</span>
        <label class="consumer-checkbox">
          <input
            type="checkbox"
            checked={anthropicConsumerStatus.alerts_5h_enabled}
            disabled={!anthropicConsumerStatus.supports_5h_usage}
            on:change={(event) => void setConsumerWindowAlertsEnabled('anthropic', '5h', (event.currentTarget as HTMLInputElement).checked)}
            on:mousedown|stopPropagation
          />
          <span>5h</span>
        </label>
        <label class="consumer-checkbox">
          <input
            type="checkbox"
            checked={anthropicConsumerStatus.alerts_week_enabled}
            disabled={!anthropicConsumerStatus.supports_week_usage}
            on:change={(event) => void setConsumerWindowAlertsEnabled('anthropic', 'week', (event.currentTarget as HTMLInputElement).checked)}
            on:mousedown|stopPropagation
          />
          <span>{anthropicConsumerStatus.supports_week_usage ? 'Week' : 'Week unavailable locally'}</span>
        </label>
      </div>
      {#if anthropicConsumerStatus.status_message}
        <span class="field-help">{anthropicConsumerStatus.status_message}</span>
      {/if}

      <div class="status-row">
        {#if errorMessage}
          <span class="status error">{errorMessage}</span>
        {:else}
          <span class="status"></span>
        {/if}
      </div>
    </div>
  </div>
</div>

<style>
  .shell {
    position: absolute;
    inset: 0;
    padding: 8px;
    background: transparent;
  }

  .panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    border: 1px solid var(--border-panel);
    border-radius: 12px;
    background: var(--bg-surface);
    color: var(--text-hi);
    font-size: 12px;
    overflow: hidden;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 36px;
    padding: 0 8px 0 12px;
    border-bottom: 1px solid var(--divider-color);
    cursor: grab;
    user-select: none;
    -webkit-user-select: none;
    flex-shrink: 0;
  }

  .bar:active {
    cursor: grabbing;
  }

  .bar-title {
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.01em;
    color: var(--text-hi);
  }

  .bar-spacer {
    flex: 1;
  }

  .bar-btn {
    width: 22px;
    height: 22px;
    border: 1px solid var(--border-btn);
    border-radius: 999px;
    background: var(--surface-btn);
    color: var(--text-mid);
    font-size: 12px;
    line-height: 1;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .bar-btn:hover {
    background: var(--surface-btn-hover);
  }

  .body {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow: hidden;
    padding: 12px;
    gap: 8px;
  }

  .placeholder {
    font-size: 11px;
    color: var(--text-lo);
  }

  .field-help {
    font-size: 10px;
    line-height: 1.35;
    color: var(--text-lo);
  }

  .consumer-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid var(--border-card);
    border-radius: 8px;
    background: var(--surface-row);
    flex-shrink: 0;
  }

  .consumer-subrow {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 16px;
    margin: -2px 0 4px 14px;
    padding: 0 2px;
    flex-shrink: 0;
    flex-wrap: wrap;
  }

  .consumer-subrow-label {
    font-size: 10px;
    color: var(--text-lo);
    line-height: 1;
  }

  .consumer-checkbox {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-lo);
    font-size: 10px;
    line-height: 1;
    cursor: pointer;
    user-select: none;
    -webkit-user-select: none;
  }

  .consumer-checkbox input {
    width: 11px;
    height: 11px;
    margin: 0;
    accent-color: rgba(110, 140, 230, 0.9);
    cursor: pointer;
  }

  .consumer-provider-label {
    flex: 1;
    font-size: 12px;
    color: var(--text-mid);
  }

  .consumer-name {
    flex: 1;
    border: none;
    background: transparent;
    color: var(--text-hi);
    font: inherit;
    font-size: 12px;
    font-weight: 600;
    outline: none;
    padding: 0;
    min-width: 0;
  }

  .consumer-name:focus {
    border-bottom: 1px solid rgba(100, 140, 255, 0.4);
  }

  .static-label {
    display: block;
  }

  .account-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
    flex-shrink: 0;
  }

  .status-row {
    display: flex;
    align-items: center;
    min-height: 14px;
    padding-top: 4px;
  }

  .status {
    font-size: 10px;
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .status.error {
    color: rgba(255, 170, 170, 0.95);
  }

</style>
