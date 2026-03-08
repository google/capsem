<script lang="ts">
  import { onMount } from 'svelte';
  import Sidebar from './Sidebar.svelte';
  import DownloadProgress from './DownloadProgress.svelte';
  import TerminalView from '../views/TerminalView.svelte';
  import StatsView from '../views/StatsView.svelte';
  import SettingsView from '../views/SettingsView.svelte';
  import WizardView from '../views/WizardView.svelte';
  import { sidebarStore } from '../stores/sidebar.svelte';
  import { settingsStore } from '../stores/settings.svelte';
  import { themeStore } from '../stores/theme.svelte';
  import { vmStore } from '../stores/vm.svelte';

  let checkedFirstRun = $state(false);

  onMount(() => {
    themeStore.init();
    vmStore.init();
    settingsStore.load();

    const handleKeydown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === ',') {
        e.preventDefault();
        const nextView = sidebarStore.activeView === 'settings' ? 'terminal' : 'settings';
        sidebarStore.setView(nextView);
      }
    };
    window.addEventListener('keydown', handleKeydown);
    return () => window.removeEventListener('keydown', handleKeydown);
  });

  $effect(() => {
    if (!checkedFirstRun && !settingsStore.loading && settingsStore.tree.length > 0) {
      checkedFirstRun = true;
    }
  });
</script>

<div class="flex h-screen w-screen overflow-hidden bg-base-100 text-base-content">
  <Sidebar />
  <div class="flex flex-1 flex-col min-w-0">
    {#if vmStore.isDownloading}
      <DownloadProgress />
    {:else}
      <!-- Content area: terminal is always mounted, hidden with visibility to avoid refit flash -->
      <div class="flex-1 min-h-0 relative">
        <div
          class="absolute inset-0"
          style:visibility={sidebarStore.activeView === 'terminal' ? 'visible' : 'hidden'}
        >
          <TerminalView />
        </div>
        {#if sidebarStore.activeView === 'stats'}
          <StatsView />
        {:else if sidebarStore.activeView === 'settings'}
          <SettingsView />
        {:else if sidebarStore.activeView === 'wizard'}
          <WizardView />
        {/if}
      </div>
    {/if}
  </div>
</div>
