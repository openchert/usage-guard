<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { currentWindow, invoke, listen } from './tauri';
  import { providerMeta } from './providerTheme';

  interface ProviderCatalogEntry {
    id: string;
    label: string;
  }

  interface ProviderAccountView {
    id: string;
    provider: string;
    provider_label: string;
    label: string;
    has_api_key: boolean;
  }

  interface ProviderSettingsPayload {
    providers: ProviderCatalogEntry[];
    accounts: ProviderAccountView[];
  }

  interface ProviderForm {
    id: string | null;
    provider: string;
    label: string;
    apiKey: string;
  }

  interface OAuthStatus {
    connected: boolean;
    plan_type: string | null;
    label: string | null;
    alerts_5h_enabled: boolean;
    alerts_week_enabled: boolean;
  }

  let providers = [] as ProviderCatalogEntry[];
  let accounts = [] as ProviderAccountView[];
  let form = {
    id: null,
    provider: '',
    label: '',
    apiKey: '',
  } as ProviderForm;
  let isLoading = true;
  let isSaving = false;
  let savePhase = 'idle' as 'idle' | 'saving' | 'verifying';
  let isTestingAlert = false;
  let isConnectingOpenAi = false;
  let isConnectingAnthropic = false;
  let errorMessage = '';
  let successMessage = '';
  let openaiOAuthStatus: OAuthStatus = { connected: false, plan_type: null, label: null, alerts_5h_enabled: true, alerts_week_enabled: true };
  let anthropicOAuthStatus: OAuthStatus = { connected: false, plan_type: null, label: null, alerts_5h_enabled: true, alerts_week_enabled: true };
  let openaiEditLabel = '';
  let anthropicEditLabel = '';
  let confirmRemoveId: string | null = null;
  let editingAccount: ProviderAccountView | null = null;
  let unlistenTheme: (() => void) | null = null;

  const REFRESH_EVENT = 'usageguard://refresh';

  async function applyTheme(): Promise<void> {
    if (!invoke) return;
    try {
      const cfg = await invoke('get_config') as { light_mode: boolean };
      document.documentElement.classList.toggle('light-mode', cfg.light_mode);
    } catch { /* ignore */ }
  }

  function defaultOpenAILabel(status: OAuthStatus): string {
    return status.label || (status.plan_type ? `ChatGPT ${status.plan_type}` : 'ChatGPT');
  }

  function defaultAnthropicLabel(status: OAuthStatus): string {
    return status.label || (status.plan_type ? `Claude ${status.plan_type}` : 'Claude');
  }

  function apiAccountSectionLabel(): string {
    return 'Organization API accounts';
  }

  function apiKeyLabel(providerId: string): string {
    switch (providerId) {
      case 'openai':
        return 'Organization API key';
      case 'anthropic':
        return 'Admin API key';
      default:
        return 'API key';
    }
  }

  function apiKeyPlaceholder(providerId: string): string {
    if (form.id) return 'Leave blank to keep current';
    switch (providerId) {
      case 'openai':
        return 'Paste organization monitoring key';
      case 'anthropic':
        return 'Paste sk-ant-admin... key';
      default:
        return 'Paste key';
    }
  }

  function apiKeyHelp(providerId: string): string {
    const existingKeyHint = form.id ? ' Leave blank to keep the current key.' : '';
    switch (providerId) {
      case 'openai':
        return `Only organization usage keys are supported. UsageGuard verifies both OpenAI organization cost and organization usage access before saving.${existingKeyHint}`;
      case 'anthropic':
        return `Only organization monitoring keys are supported. Anthropic requires an Admin API key (\`sk-ant-admin...\`) for usage and cost reports.${existingKeyHint}`;
      default:
        return `Stored locally and used only for built-in audited organization usage endpoints.${existingKeyHint}`;
    }
  }

  function saveButtonLabel(): string {
    if (!isSaving) return form.id ? 'Save' : 'Add';
    return savePhase === 'verifying' ? 'Verifying...' : 'Saving...';
  }

  function resetForm(providerId = form.provider || providers[0]?.id || ''): void {
    form = { id: null, provider: providerId, label: '', apiKey: '' };
    errorMessage = '';
    successMessage = '';
  }

  function applyPayload(payload: ProviderSettingsPayload): void {
    providers = payload.providers;
    accounts = payload.accounts;
    if (!providers.some((p) => p.id === form.provider)) {
      form.provider = providers[0]?.id ?? '';
    }
  }

  async function closeWindow(): Promise<void> {
    if (!currentWindow) return;
    try { await currentWindow.close(); } catch { /* ignore */ }
  }

  async function startDrag(event: MouseEvent): Promise<void> {
    if (!currentWindow || event.button !== 0) return;
    try { await currentWindow.startDragging(); } catch { /* ignore */ }
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    void closeWindow();
  }

  function beginEdit(account: ProviderAccountView): void {
    errorMessage = '';
    successMessage = '';
    form = {
      id: account.id,
      provider: account.provider,
      label: account.label,
      apiKey: '',
    };
  }

  async function loadSettings(): Promise<void> {
    if (!invoke) return;
    isLoading = true;
    try {
      const [payload, openaiStatus, anthropicStatus] = await Promise.all([
        invoke('get_provider_settings') as Promise<ProviderSettingsPayload>,
        invoke('get_openai_oauth_status') as Promise<OAuthStatus>,
        invoke('get_anthropic_oauth_status') as Promise<OAuthStatus>,
      ]);
      applyPayload(payload);
      openaiOAuthStatus = openaiStatus;
      anthropicOAuthStatus = anthropicStatus;
      openaiEditLabel = defaultOpenAILabel(openaiStatus);
      anthropicEditLabel = defaultAnthropicLabel(anthropicStatus);
      if (!form.provider) resetForm(payload.providers[0]?.id ?? '');
    } catch (error) {
      errorMessage = String(error);
    } finally {
      isLoading = false;
    }
  }

  async function saveOAuthLabel(provider: 'openai' | 'anthropic', label: string): Promise<void> {
    if (!invoke) return;
    try {
      await invoke('set_oauth_label', { provider, label });
    } catch (error) {
      errorMessage = String(error);
    }
  }

  async function setOAuthWindowAlertsEnabled(
    provider: 'openai' | 'anthropic',
    windowKey: '5h' | 'week',
    enabled: boolean,
  ): Promise<void> {
    if (!invoke) return;
    const target = provider === 'openai' ? openaiOAuthStatus : anthropicOAuthStatus;
    const field = windowKey === '5h' ? 'alerts_5h_enabled' : 'alerts_week_enabled';
    const previous = target[field];

    if (provider === 'openai') {
      openaiOAuthStatus = { ...openaiOAuthStatus, [field]: enabled };
    } else {
      anthropicOAuthStatus = { ...anthropicOAuthStatus, [field]: enabled };
    }

    try {
      await invoke('set_oauth_window_alerts_enabled', { provider, windowKey, enabled });
    } catch (error) {
      if (provider === 'openai') {
        openaiOAuthStatus = { ...openaiOAuthStatus, [field]: previous };
      } else {
        anthropicOAuthStatus = { ...anthropicOAuthStatus, [field]: previous };
      }
      errorMessage = String(error);
    }
  }

  async function sendTestAlert(provider: string, accountLabel: string): Promise<void> {
    if (!invoke || isTestingAlert) return;
    isTestingAlert = true;
    errorMessage = '';
    successMessage = '';
    try {
      const testedLabel = await invoke('send_test_alert', {
        target: {
          provider,
          accountLabel,
        },
      }) as string;
      successMessage = `Test alert sent for ${testedLabel}.`;
    } catch (error) {
      errorMessage = String(error);
    } finally {
      isTestingAlert = false;
    }
  }

  async function connectOpenAIOAuth(): Promise<void> {
    if (!invoke || isConnectingOpenAi) return;
    isConnectingOpenAi = true;
    errorMessage = '';
    successMessage = '';
    try {
      const planType = await invoke('connect_openai_oauth') as string;
      openaiOAuthStatus = { ...openaiOAuthStatus, connected: true, plan_type: planType, label: null };
      openaiEditLabel = `ChatGPT ${planType}`;
    } catch (error) {
      errorMessage = String(error);
    } finally {
      isConnectingOpenAi = false;
    }
  }

  async function disconnectOpenAIOAuth(): Promise<void> {
    if (!invoke) return;
    try {
      await invoke('disconnect_openai_oauth');
      openaiOAuthStatus = { ...openaiOAuthStatus, connected: false, plan_type: null, label: null };
      openaiEditLabel = '';
    } catch (error) {
      errorMessage = String(error);
    }
  }

  async function connectAnthropicOAuth(): Promise<void> {
    if (!invoke || isConnectingAnthropic) return;
    isConnectingAnthropic = true;
    errorMessage = '';
    successMessage = '';
    try {
      const planType = await invoke('connect_anthropic_oauth') as string;
      anthropicOAuthStatus = { ...anthropicOAuthStatus, connected: true, plan_type: planType, label: null };
      anthropicEditLabel = `Claude ${planType}`;
    } catch (error) {
      errorMessage = String(error);
    } finally {
      isConnectingAnthropic = false;
    }
  }

  async function disconnectAnthropicOAuth(): Promise<void> {
    if (!invoke) return;
    try {
      await invoke('disconnect_anthropic_oauth');
      anthropicOAuthStatus = { ...anthropicOAuthStatus, connected: false, plan_type: null, label: null };
      anthropicEditLabel = '';
    } catch (error) {
      errorMessage = String(error);
    }
  }

  async function save(): Promise<void> {
    if (!invoke || isSaving) return;
    isSaving = true;
    savePhase = form.apiKey.trim() ? 'verifying' : 'saving';
    errorMessage = '';
    successMessage = '';
    const editing = Boolean(form.id);
    try {
      const payload = await invoke('save_provider_account', {
        input: {
          id: form.id ?? undefined,
          provider: form.provider,
          label: form.label,
          apiKey: form.apiKey,
        },
      }) as ProviderSettingsPayload;
      applyPayload(payload);
      successMessage = editing ? 'Saved.' : 'Added.';
      resetForm(form.provider);
    } catch (error) {
      errorMessage = String(error);
    } finally {
      isSaving = false;
      savePhase = 'idle';
    }
  }

  async function removeAccount(account: ProviderAccountView): Promise<void> {
    if (!invoke) return;
    errorMessage = '';
    successMessage = '';
    try {
      const payload = await invoke('delete_provider_account', { id: account.id }) as ProviderSettingsPayload;
      applyPayload(payload);
      confirmRemoveId = null;
      if (form.id === account.id) resetForm(form.provider);
    } catch (error) {
      errorMessage = String(error);
    }
  }

  onMount(async () => {
    window.addEventListener('keydown', onKeydown);
    void applyTheme();
    void loadSettings();
    if (listen) {
      unlistenTheme = await listen(REFRESH_EVENT, () => {
        void applyTheme();
      });
    }
  });

  onDestroy(() => {
    window.removeEventListener('keydown', onKeydown);
    unlistenTheme?.();
  });

  $: editingAccount = form.id ? accounts.find((account) => account.id === form.id) ?? null : null;
</script>

<div class="shell" on:contextmenu|preventDefault role="presentation">
  <div class="panel">
    <!-- Title bar -->
    <header class="bar" on:mousedown={startDrag} role="presentation">
      <span class="bar-title">Providers</span>
      <div class="bar-spacer"></div>
      <button class="bar-btn" type="button" title="New provider" on:mousedown|stopPropagation on:click|stopPropagation={() => resetForm()}>+</button>
      <button class="bar-btn bar-btn-close" type="button" title="Close" on:mousedown|stopPropagation on:click|stopPropagation={closeWindow}>×</button>
    </header>

    <div class="body">

      <!-- Subscription connections -->
      <div class="section-label">Subscriptions</div>

      <!-- ChatGPT OAuth -->
      <div class="oauth-row" class:oauth-connected={openaiOAuthStatus.connected}>
        <div class="account-dot" style="--accent:{openaiOAuthStatus.connected ? '#10a37f' : 'rgba(130, 138, 165, 0.35)'}"></div>
        {#if isConnectingOpenAi}
          <span class="account-vendor" style="flex:1">Waiting for browser…</span>
        {:else if openaiOAuthStatus.connected}
          <input
            class="oauth-name"
            type="text"
            bind:value={openaiEditLabel}
            on:blur={() => saveOAuthLabel('openai', openaiEditLabel)}
            on:mousedown|stopPropagation
          />
          <button class="link-btn" type="button" on:click={disconnectOpenAIOAuth}>Disconnect</button>
        {:else}
          <span class="oauth-provider-label">ChatGPT</span>
          <button class="connect-btn" type="button" on:click={connectOpenAIOAuth}>Connect</button>
        {/if}
      </div>
      {#if openaiOAuthStatus.connected && !isConnectingOpenAi}
        <div class="oauth-subrow">
          <span class="oauth-subrow-label">Alerts</span>
          <label class="oauth-checkbox">
            <input
              type="checkbox"
              checked={openaiOAuthStatus.alerts_5h_enabled}
              on:change={(event) => void setOAuthWindowAlertsEnabled('openai', '5h', (event.currentTarget as HTMLInputElement).checked)}
              on:mousedown|stopPropagation
            />
            <span>5h</span>
          </label>
          <label class="oauth-checkbox">
            <input
              type="checkbox"
              checked={openaiOAuthStatus.alerts_week_enabled}
              on:change={(event) => void setOAuthWindowAlertsEnabled('openai', 'week', (event.currentTarget as HTMLInputElement).checked)}
              on:mousedown|stopPropagation
            />
            <span>Week</span>
          </label>
          <button
            class="link-btn alert-test-button"
            type="button"
            disabled={isTestingAlert}
            on:mousedown|stopPropagation
            on:click={() => void sendTestAlert('openai', defaultOpenAILabel(openaiOAuthStatus))}
          >
            Test alert
          </button>
        </div>
      {/if}
      {#if isConnectingOpenAi}
        <span class="field-help">Verifying ChatGPT subscription...</span>
      {/if}

      <!-- Claude OAuth -->
      <div class="oauth-row" class:oauth-connected={anthropicOAuthStatus.connected}>
        <div class="account-dot" style="--accent:{anthropicOAuthStatus.connected ? '#d97a4e' : 'rgba(130, 138, 165, 0.35)'}"></div>
        {#if isConnectingAnthropic}
          <span class="account-vendor" style="flex:1">Waiting for browser…</span>
        {:else if anthropicOAuthStatus.connected}
          <input
            class="oauth-name"
            type="text"
            bind:value={anthropicEditLabel}
            on:blur={() => saveOAuthLabel('anthropic', anthropicEditLabel)}
            on:mousedown|stopPropagation
          />
          <button class="link-btn" type="button" on:click={disconnectAnthropicOAuth}>Disconnect</button>
        {:else}
          <span class="oauth-provider-label">Claude</span>
          <button class="connect-btn" type="button" on:click={connectAnthropicOAuth}>Connect</button>
        {/if}
      </div>
      {#if anthropicOAuthStatus.connected && !isConnectingAnthropic}
        <div class="oauth-subrow">
          <span class="oauth-subrow-label">Alerts</span>
          <label class="oauth-checkbox">
            <input
              type="checkbox"
              checked={anthropicOAuthStatus.alerts_5h_enabled}
              on:change={(event) => void setOAuthWindowAlertsEnabled('anthropic', '5h', (event.currentTarget as HTMLInputElement).checked)}
              on:mousedown|stopPropagation
            />
            <span>5h</span>
          </label>
          <label class="oauth-checkbox">
            <input
              type="checkbox"
              checked={anthropicOAuthStatus.alerts_week_enabled}
              on:change={(event) => void setOAuthWindowAlertsEnabled('anthropic', 'week', (event.currentTarget as HTMLInputElement).checked)}
              on:mousedown|stopPropagation
            />
            <span>Week</span>
          </label>
          <button
            class="link-btn alert-test-button"
            type="button"
            disabled={isTestingAlert}
            on:mousedown|stopPropagation
            on:click={() => void sendTestAlert('anthropic', defaultAnthropicLabel(anthropicOAuthStatus))}
          >
            Test alert
          </button>
        </div>
      {/if}
      {#if isConnectingAnthropic}
        <span class="field-help">Verifying Claude subscription...</span>
      {/if}

      <div class="divider"></div>

      <!-- API key accounts -->
      <div class="section-label">{apiAccountSectionLabel()}</div>

      {#if isLoading}
        <div class="placeholder">Loading…</div>
      {:else if accounts.length === 0}
        <div class="placeholder">No accounts yet.</div>
      {:else}
        <div class="account-list">
          {#each accounts as account}
            {@const meta = providerMeta(account.provider)}
            <div
              class="account-row"
              class:active={form.id === account.id}
              class:confirming={confirmRemoveId === account.id}
              style="--accent:{meta.color}"
              role="button"
              tabindex="0"
              on:click={() => { if (confirmRemoveId !== account.id) beginEdit(account); }}
              on:keydown={(e) => e.key === 'Enter' && confirmRemoveId !== account.id && beginEdit(account)}
            >
              <div class="account-dot"></div>
              {#if confirmRemoveId === account.id}
                <span class="confirm-label">Remove "{account.label}"?</span>
                <button class="confirm-yes" type="button" on:mousedown|stopPropagation on:click|stopPropagation={() => removeAccount(account)}>Remove</button>
                <button class="confirm-no" type="button" on:mousedown|stopPropagation on:click|stopPropagation={() => confirmRemoveId = null}>Cancel</button>
              {:else}
                <div class="account-info">
                  <span class="account-name">{account.label}</span>
                  <span class="account-vendor">{account.provider_label}</span>
                  {#if !account.has_api_key}
                    <span class="account-warn">no key</span>
                  {/if}
                </div>
                <button class="row-remove" type="button" title="Remove" on:mousedown|stopPropagation on:click|stopPropagation={() => confirmRemoveId = account.id}>×</button>
              {/if}
            </div>
          {/each}
        </div>
      {/if}

      <div class="divider"></div>

      <!-- Add / Edit form -->
      <div class="form-head">
        <span class="form-label">{form.id ? 'Edit' : 'Add'}</span>
        {#if form.id}
          <button class="link-btn" type="button" on:click={() => resetForm(form.provider)}>Cancel</button>
        {/if}
      </div>

      <div class="form-fields">
        <div class="row-2">
          <label class="field">
            <span class="field-label">Vendor</span>
            <select bind:value={form.provider} disabled={Boolean(form.id)}>
              {#each providers as provider}
                <option value={provider.id}>{provider.label}</option>
              {/each}
            </select>
          </label>
          <label class="field">
            <span class="field-label">Name</span>
            <input type="text" bind:value={form.label} placeholder="Work, Personal…" autocomplete="off" />
          </label>
        </div>
        <label class="field">
          <span class="field-label">{apiKeyLabel(form.provider)}</span>
          <input
            type="password"
            bind:value={form.apiKey}
            placeholder={apiKeyPlaceholder(form.provider)}
            autocomplete="off"
          />
        </label>
        <span class="field-help">{apiKeyHelp(form.provider)}</span>
      </div>

      <div class="form-footer">
        {#if errorMessage}
          <span class="status error">{errorMessage}</span>
        {:else if isSaving}
          <span class="status">{savePhase === 'verifying' ? 'Verifying...' : 'Saving...'}</span>
        {:else if successMessage}
          <span class="status success">{successMessage}</span>
        {:else}
          <span class="status"></span>
        {/if}
        <div class="footer-actions">
          {#if editingAccount?.has_api_key}
            <button
              class="link-btn alert-test-button"
              type="button"
              disabled={isTestingAlert}
              on:click={() => void sendTestAlert(editingAccount.provider, editingAccount.label)}
            >
              Test alert
            </button>
          {/if}
          <button class="save-btn" type="button" disabled={isSaving} title={saveButtonLabel()} aria-label={saveButtonLabel()} on:click={save}>
          {isSaving ? '…' : form.id ? 'Save' : 'Add'}
          </button>
        </div>
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

  /* Title bar */
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
  .bar:active { cursor: grabbing; }

  .bar-title {
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.01em;
    color: var(--text-hi);
  }

  .bar-spacer { flex: 1; }

  .bar-btn {
    width: 22px;
    height: 22px;
    border: 1px solid var(--border-btn);
    border-radius: 999px;
    background: var(--surface-btn);
    color: var(--text-mid);
    font-size: 14px;
    line-height: 1;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .bar-btn:hover { background: var(--surface-btn-hover); }
  .bar-btn-close { font-size: 16px; }

  /* Body */
  .body {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow: hidden;
    padding: 10px;
    gap: 6px;
  }

  /* Section labels */
  .section-label {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    color: var(--text-lo);
    flex-shrink: 0;
  }

  .field-help {
    font-size: 10px;
    line-height: 1.35;
    color: var(--text-lo);
  }

  /* OAuth rows */
  .oauth-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border: 1px solid var(--border-card);
    border-radius: 8px;
    background: var(--surface-row);
    flex-shrink: 0;
  }

  .oauth-subrow {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 16px;
    margin: -2px 0 2px 14px;
    padding: 0 2px;
    flex-shrink: 0;
  }

  .oauth-subrow-label {
    font-size: 10px;
    color: var(--text-lo);
    line-height: 1;
  }

  .oauth-checkbox {
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

  .oauth-checkbox input {
    width: 11px;
    height: 11px;
    margin: 0;
    accent-color: rgba(110, 140, 230, 0.9);
    cursor: pointer;
  }

  .oauth-provider-label {
    flex: 1;
    font-size: 12px;
    color: var(--text-mid);
  }

  .oauth-name {
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
  .oauth-name:focus {
    border-bottom: 1px solid rgba(100, 140, 255, 0.4);
  }

  .connect-btn {
    padding: 4px 10px;
    border: 1px solid var(--border-btn);
    border-radius: 999px;
    background: var(--surface-btn);
    color: var(--text-hi);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
    flex-shrink: 0;
  }
  .connect-btn:hover { background: var(--surface-btn-hover); }

  /* Account list */
  .placeholder {
    font-size: 11px;
    color: var(--text-lo);
    padding: 4px 0;
  }

  .account-list {
    display: flex;
    flex-direction: column;
    gap: 3px;
    overflow-y: auto;
    max-height: 130px;
    flex-shrink: 0;
  }

  .account-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border: 1px solid var(--border-card);
    border-radius: 8px;
    background: var(--surface-row);
    cursor: pointer;
    transition: background 0.1s;
  }
  .account-row:hover, .account-row.active {
    background: var(--surface-row-hover);
    border-color: var(--border-row-hover);
  }

  .account-row.confirming {
    border-color: rgba(255, 100, 100, 0.25);
    background: rgba(255, 80, 80, 0.06);
  }

  .confirm-label {
    flex: 1;
    font-size: 11px;
    color: rgba(220, 180, 180, 0.9);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .confirm-yes {
    padding: 3px 8px;
    border: 1px solid rgba(255, 100, 100, 0.35);
    border-radius: 999px;
    background: rgba(255, 80, 80, 0.12);
    color: rgba(255, 160, 160, 0.95);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
    flex-shrink: 0;
  }
  .confirm-yes:hover { background: rgba(255, 80, 80, 0.22); }

  .confirm-no {
    padding: 3px 8px;
    border: 1px solid var(--border-btn);
    border-radius: 999px;
    background: transparent;
    color: var(--text-lo);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
    flex-shrink: 0;
  }
  .confirm-no:hover { color: var(--text-mid); }

  .account-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--accent);
    flex-shrink: 0;
  }

  .account-info {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
  }

  .account-name {
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text-hi);
  }

  .account-vendor {
    font-size: 10px;
    color: var(--text-lo);
    white-space: nowrap;
  }

  .account-warn {
    font-size: 10px;
    color: rgba(240, 160, 120, 0.9);
    white-space: nowrap;
  }

  .row-remove {
    width: 18px;
    height: 18px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: rgba(180, 120, 120, 0.6);
    font-size: 14px;
    line-height: 1;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    opacity: 0;
    transition: opacity 0.1s;
  }
  .account-row:hover .row-remove { opacity: 1; }
  .row-remove:hover { color: #ff9999; background: rgba(255, 100, 100, 0.1); }

  /* Divider */
  .divider {
    height: 1px;
    background: var(--divider-color);
    flex-shrink: 0;
  }

  /* Form */
  .form-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-shrink: 0;
  }

  .form-label {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    color: var(--text-lo);
  }

  .link-btn {
    border: none;
    background: none;
    color: var(--text-lo);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
    padding: 0;
  }
  .link-btn:hover { color: var(--text-mid); }
  .link-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .link-btn:disabled:hover { color: var(--text-lo); }

  .alert-test-button {
    display: none;
  }

  .form-fields {
    display: flex;
    flex-direction: column;
    gap: 6px;
    flex: 1;
  }

  .row-2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .field-label {
    font-size: 10px;
    color: var(--text-lo);
  }

  input:not([type='checkbox']),
  select {
    width: 100%;
    padding: 6px 9px;
    border: 1px solid var(--border-input);
    border-radius: 8px;
    background: var(--surface-input);
    color: var(--text-hi);
    font: inherit;
    font-size: 12px;
    outline: none;
    box-sizing: border-box;
  }
  input:not([type='checkbox']):focus, select:focus {
    border-color: rgba(100, 140, 255, 0.45);
    background: var(--surface-input-focus);
  }

  select {
    appearance: none;
    -webkit-appearance: none;
    padding-right: 24px;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='6' viewBox='0 0 10 6'%3E%3Cpath d='M1 1l4 4 4-4' stroke='rgba(160,168,192,0.7)' stroke-width='1.5' fill='none' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 8px center;
    cursor: pointer;
  }

  :global(.light-mode) select {
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='6' viewBox='0 0 10 6'%3E%3Cpath d='M1 1l4 4 4-4' stroke='rgba(80,100,160,0.6)' stroke-width='1.5' fill='none' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E");
  }

  /* Footer */
  .form-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    flex-shrink: 0;
    padding-top: 2px;
  }

  .footer-actions {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-shrink: 0;
  }

  .status {
    font-size: 10px;
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .status.error { color: rgba(255, 170, 170, 0.95); }
  .status.success { color: rgba(160, 230, 190, 0.95); }

  .save-btn {
    padding: 6px 16px;
    border: 1px solid var(--border-btn);
    border-radius: 999px;
    background: var(--surface-btn);
    color: var(--text-hi);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    flex-shrink: 0;
    transition: background 0.1s;
  }
  .save-btn:hover:not(:disabled) { background: var(--surface-btn-hover); }
  .save-btn:disabled { opacity: 0.5; cursor: default; }
</style>
