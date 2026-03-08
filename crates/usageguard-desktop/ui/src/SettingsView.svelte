<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { currentWindow, invoke } from './tauri';
  import { providerMeta } from './providerTheme';

  interface ProviderCatalogEntry {
    id: string;
    label: string;
    default_endpoint: string | null;
    endpoint_required: boolean;
    endpoint_hint: string;
  }

  interface ProviderAccountView {
    id: string;
    provider: string;
    provider_label: string;
    label: string;
    endpoint: string | null;
    default_endpoint: string | null;
    has_api_key: boolean;
    endpoint_required: boolean;
  }

  interface ProviderSettingsPayload {
    providers: ProviderCatalogEntry[];
    accounts: ProviderAccountView[];
  }

  interface ProviderForm {
    id: string | null;
    provider: string;
    label: string;
    endpoint: string;
    apiKey: string;
  }

  interface OAuthStatus {
    connected: boolean;
    plan_type: string | null;
  }

  let providers = [] as ProviderCatalogEntry[];
  let accounts = [] as ProviderAccountView[];
  let form = {
    id: null,
    provider: '',
    label: '',
    endpoint: '',
    apiKey: '',
  } as ProviderForm;
  let isLoading = true;
  let isSaving = false;
  let isConnecting = false;
  let errorMessage = '';
  let successMessage = '';
  let oauthStatus: OAuthStatus = { connected: false, plan_type: null };

  function resetForm(providerId = form.provider || providers[0]?.id || ''): void {
    form = {
      id: null,
      provider: providerId,
      label: '',
      endpoint: '',
      apiKey: '',
    };
    errorMessage = '';
    successMessage = '';
  }

  function applyPayload(payload: ProviderSettingsPayload): void {
    providers = payload.providers;
    accounts = payload.accounts;

    if (!providers.some((provider) => provider.id === form.provider)) {
      form.provider = providers[0]?.id ?? '';
    }
  }

  function selectedProvider(): ProviderCatalogEntry | null {
    return providers.find((provider) => provider.id === form.provider) ?? providers[0] ?? null;
  }

  function endpointPlaceholder(): string {
    return selectedProvider()?.default_endpoint ?? 'https://api.vendor.example/usage';
  }

  function endpointHint(): string {
    const provider = selectedProvider();
    if (!provider) return '';
    if (provider.endpoint_required) return 'Required';
    if (provider.default_endpoint) return `Default: ${provider.default_endpoint}`;
    return provider.endpoint_hint;
  }

  function needsEndpoint(): boolean {
    const p = selectedProvider();
    return p ? p.endpoint_required || !p.default_endpoint : true;
  }

  async function closeWindow(): Promise<void> {
    if (!currentWindow) return;
    try {
      await currentWindow.close();
    } catch (error) {
      console.error('close settings failed:', error);
    }
  }

  async function startDrag(event: MouseEvent): Promise<void> {
    if (!currentWindow || event.button !== 0) return;
    try {
      await currentWindow.startDragging();
    } catch {
      // Ignore rejected drag gestures.
    }
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
      endpoint: account.endpoint ?? '',
      apiKey: '',
    };
  }

  async function loadSettings(): Promise<void> {
    if (!invoke) return;

    isLoading = true;
    try {
      const [payload, status] = await Promise.all([
        invoke('get_provider_settings') as Promise<ProviderSettingsPayload>,
        invoke('get_openai_oauth_status') as Promise<OAuthStatus>,
      ]);
      applyPayload(payload);
      oauthStatus = status;
      if (!form.provider) resetForm(payload.providers[0]?.id ?? '');
    } catch (error) {
      errorMessage = String(error);
    } finally {
      isLoading = false;
    }
  }

  async function connectOAuth(): Promise<void> {
    if (!invoke || isConnecting) return;
    isConnecting = true;
    errorMessage = '';
    successMessage = '';
    try {
      const planType = await invoke('connect_openai_oauth') as string;
      oauthStatus = { connected: true, plan_type: planType };
      successMessage = `ChatGPT ${planType} connected.`;
    } catch (error) {
      errorMessage = String(error);
    } finally {
      isConnecting = false;
    }
  }

  async function disconnectOAuth(): Promise<void> {
    if (!invoke) return;
    try {
      await invoke('disconnect_openai_oauth');
      oauthStatus = { connected: false, plan_type: null };
    } catch (error) {
      errorMessage = String(error);
    }
  }

  async function debugOAuth(): Promise<void> {
    if (!invoke) return;
    const result = await invoke('debug_openai_oauth') as string;
    try {
      await navigator.clipboard.writeText(result);
      successMessage = 'Debug output copied to clipboard.';
    } catch {
      // Clipboard not available — fall back to alert
      window.alert(result);
    }
  }

  async function save(): Promise<void> {
    if (!invoke || isSaving) return;

    isSaving = true;
    errorMessage = '';
    successMessage = '';
    const editing = Boolean(form.id);

    try {
      const payload = await invoke('save_provider_account', {
        input: {
          id: form.id ?? undefined,
          provider: form.provider,
          label: form.label,
          endpoint: form.endpoint,
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
    }
  }

  async function removeAccount(account: ProviderAccountView): Promise<void> {
    if (!invoke) return;
    if (!window.confirm(`Remove '${account.label}'?`)) return;

    errorMessage = '';
    successMessage = '';

    try {
      const payload = await invoke('delete_provider_account', {
        id: account.id,
      }) as ProviderSettingsPayload;
      applyPayload(payload);
      successMessage = 'Removed.';
      if (form.id === account.id) resetForm(form.provider);
    } catch (error) {
      errorMessage = String(error);
    }
  }

  onMount(() => {
    window.addEventListener('keydown', onKeydown);
    void loadSettings();
  });

  onDestroy(() => {
    window.removeEventListener('keydown', onKeydown);
  });
</script>

<div class="shell">
  <div class="panel">
    <!-- Title bar -->
    <header class="bar" on:mousedown={startDrag} role="presentation">
      <span class="bar-title">Providers</span>
      {#if !isLoading}
        <span class="bar-count">{accounts.length}</span>
      {/if}
      <div class="bar-spacer"></div>
      <button
        class="bar-btn"
        type="button"
        title="New provider"
        on:mousedown|stopPropagation
        on:click|stopPropagation={() => resetForm()}
      >+</button>
      <button
        class="bar-btn bar-btn-close"
        type="button"
        title="Close"
        on:mousedown|stopPropagation
        on:click|stopPropagation={closeWindow}
      >×</button>
    </header>

    <div class="body">
      <!-- ChatGPT OAuth connection -->
      <div class="oauth-row" class:oauth-connected={oauthStatus.connected}>
        <div class="account-dot" style="--accent:{oauthStatus.connected ? '#10a37f' : 'rgba(255,255,255,0.2)'}"></div>
        {#if oauthStatus.connected}
          <div class="account-info">
            <span class="account-name">ChatGPT {oauthStatus.plan_type ?? ''}</span>
            <span class="account-vendor">Subscription</span>
          </div>
          <button class="link-btn" type="button" on:click={debugOAuth} title="Show raw API response">Debug</button>
          <button class="link-btn" type="button" on:click={disconnectOAuth}>Disconnect</button>
        {:else if isConnecting}
          <div class="account-info">
            <span class="account-vendor">Waiting for browser sign-in…</span>
          </div>
        {:else}
          <div class="account-info">
            <span class="account-vendor">ChatGPT Plus / Pro subscription</span>
          </div>
          <button class="save-btn" type="button" on:click={connectOAuth}>Sign in</button>
        {/if}
      </div>

      <div class="divider"></div>

      <!-- Account list -->
      {#if isLoading}
        <div class="placeholder">Loading…</div>
      {:else if accounts.length === 0}
        <div class="placeholder">No providers yet. Press + to add one.</div>
      {:else}
        <div class="account-list">
          {#each accounts as account}
            {@const meta = providerMeta(account.provider)}
            <div
              class="account-row"
              class:active={form.id === account.id}
              style="--accent:{meta.color}"
              role="button"
              tabindex="0"
              on:click={() => beginEdit(account)}
              on:keydown={(e) => e.key === 'Enter' && beginEdit(account)}
            >
              <div class="account-dot"></div>
              <div class="account-info">
                <span class="account-name">{account.label}</span>
                <span class="account-vendor">{account.provider_label}</span>
                {#if !account.has_api_key}
                  <span class="account-warn">no key</span>
                {/if}
              </div>
              <button
                class="row-remove"
                type="button"
                title="Remove"
                on:mousedown|stopPropagation
                on:click|stopPropagation={() => removeAccount(account)}
              >×</button>
            </div>
          {/each}
        </div>
      {/if}

      <!-- Divider -->
      <div class="divider"></div>

      <!-- Add / Edit form -->
      <div class="form-head">
        <span class="form-label">{form.id ? 'Edit account' : 'Add account'}</span>
        {#if form.id}
          <button class="link-btn" type="button" on:click={() => resetForm(form.provider)}>Cancel</button>
        {/if}
      </div>

      <div class="form-fields">
        <div class="row-2">
          <label class="field">
            <span class="field-label">Vendor</span>
            <select bind:value={form.provider}>
              {#each providers as provider}
                <option value={provider.id}>{provider.label}</option>
              {/each}
            </select>
          </label>

          <label class="field">
            <span class="field-label">Name</span>
            <input
              type="text"
              bind:value={form.label}
              placeholder="Work, Personal…"
              autocomplete="off"
            />
          </label>
        </div>

        <label class="field">
          <span class="field-label">API key</span>
          <input
            type="password"
            bind:value={form.apiKey}
            placeholder={form.id ? 'Leave blank to keep current' : 'Paste API key'}
            autocomplete="off"
          />
        </label>

        {#if needsEndpoint()}
          <label class="field">
            <span class="field-label">Endpoint <span class="field-hint">{endpointHint()}</span></span>
            <input
              type="url"
              bind:value={form.endpoint}
              placeholder={endpointPlaceholder()}
              autocomplete="off"
            />
          </label>
        {/if}
      </div>

      <div class="form-footer">
        <span class="status" class:error={!!errorMessage} class:success={!!successMessage && !errorMessage}>
          {errorMessage || successMessage || 'Esc closes'}
        </span>
        <button class="save-btn" type="button" disabled={isSaving} on:click={save}>
          {isSaving ? '…' : form.id ? 'Save' : 'Add'}
        </button>
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
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 14px;
    background:
      radial-gradient(circle at top right, rgba(82, 140, 255, 0.14), transparent 38%),
      linear-gradient(180deg, rgba(16, 17, 24, 0.99), rgba(9, 10, 15, 0.99));
    color: #e4e8f3;
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
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
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
    color: rgba(229, 233, 243, 0.95);
  }

  .bar-count {
    font-size: 11px;
    color: rgba(120, 128, 155, 0.9);
    font-variant-numeric: tabular-nums;
  }

  .bar-spacer { flex: 1; }

  .bar-btn {
    width: 22px;
    height: 22px;
    border: 1px solid rgba(255, 255, 255, 0.09);
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.04);
    color: rgba(210, 216, 232, 0.88);
    font-size: 14px;
    line-height: 1;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .bar-btn:hover {
    background: rgba(255, 255, 255, 0.08);
  }

  .bar-btn-close { font-size: 16px; }

  /* Body */
  .body {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow: hidden;
    padding: 10px 10px 10px;
    gap: 8px;
  }

  /* ChatGPT OAuth row */
  .oauth-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 8px;
    border: 1px solid rgba(255, 255, 255, 0.05);
    border-radius: 8px;
    background: rgba(255, 255, 255, 0.02);
    flex-shrink: 0;
  }

  /* Account list */
  .placeholder {
    font-size: 11px;
    color: rgba(120, 128, 155, 0.85);
    padding: 10px 4px;
  }

  .account-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    overflow-y: auto;
    max-height: 170px;
    flex-shrink: 0;
  }

  .account-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 8px;
    border: 1px solid rgba(255, 255, 255, 0.05);
    border-radius: 8px;
    background: rgba(255, 255, 255, 0.02);
    cursor: pointer;
    transition: background 0.1s;
  }

  .account-row:hover,
  .account-row.active {
    background: rgba(255, 255, 255, 0.05);
    border-color: rgba(255, 255, 255, 0.09);
  }

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
    color: rgba(229, 233, 243, 0.95);
  }

  .account-vendor {
    font-size: 10px;
    color: rgba(130, 138, 162, 0.88);
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
    background: rgba(255, 255, 255, 0.06);
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
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: rgba(130, 138, 162, 0.85);
  }

  .link-btn {
    border: none;
    background: none;
    color: rgba(130, 138, 162, 0.85);
    font: inherit;
    font-size: 11px;
    cursor: pointer;
    padding: 0;
  }

  .link-btn:hover { color: rgba(180, 186, 208, 0.95); }

  .form-fields {
    display: flex;
    flex-direction: column;
    gap: 7px;
    flex: 1;
  }

  .row-2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 7px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .field-label {
    font-size: 10px;
    color: rgba(150, 158, 182, 0.85);
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .field-hint {
    color: rgba(120, 128, 155, 0.7);
    font-size: 9px;
    font-style: italic;
  }

  input,
  select {
    width: 100%;
    padding: 7px 9px;
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 8px;
    background: rgba(8, 10, 14, 0.7);
    color: #e4e8f3;
    font: inherit;
    font-size: 12px;
    outline: none;
    box-sizing: border-box;
  }

  input:focus,
  select:focus {
    border-color: rgba(100, 140, 255, 0.45);
    background: rgba(10, 12, 18, 0.88);
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

  .status {
    font-size: 10px;
    color: rgba(120, 128, 155, 0.8);
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .status.error { color: rgba(255, 170, 170, 0.95); }
  .status.success { color: rgba(160, 230, 190, 0.95); }

  .save-btn {
    padding: 7px 16px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.07);
    color: rgba(229, 233, 243, 0.95);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    flex-shrink: 0;
    transition: background 0.1s;
  }

  .save-btn:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.11);
  }

  .save-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
