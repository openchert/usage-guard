<script lang="ts">
  import SettingsView from './SettingsView.svelte';
  import ProviderWidgetView from './ProviderWidgetView.svelte';

  const currentView = new URL(window.location.href).searchParams.get('view') ?? 'widget';
</script>

{#if currentView === 'settings'}
  <SettingsView />
{:else}
  <ProviderWidgetView />
{/if}
