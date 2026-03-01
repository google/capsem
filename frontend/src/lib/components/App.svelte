<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import Sidebar from './Sidebar.svelte';
  import TerminalView from '../views/TerminalView.svelte';
  import AnalyticsView from '../views/AnalyticsView.svelte';
  import SettingsView from '../views/SettingsView.svelte';
  import { sidebarStore } from '../stores/sidebar.svelte';
  import { themeStore } from '../stores/theme.svelte';
  import { vmStore } from '../stores/vm.svelte';
  import { networkStore } from '../stores/network.svelte';

  onMount(() => {
    themeStore.init();
    vmStore.init();
    networkStore.start();
  });

  onDestroy(() => {
    networkStore.stop();
  });
</script>

<div class="flex h-screen w-screen overflow-hidden bg-base-100 text-base-content">
  <Sidebar />
  <div class="flex flex-1 flex-col min-w-0">
    <!-- Content area: terminal is always mounted, hidden with visibility to avoid refit flash -->
    <div class="flex-1 min-h-0 relative">
      <div
        class="absolute inset-0"
        style:visibility={sidebarStore.activeView === 'terminal' ? 'visible' : 'hidden'}
      >
        <TerminalView />
      </div>
      {#if sidebarStore.activeView === 'analytics'}
        <AnalyticsView />
      {:else if sidebarStore.activeView === 'settings'}
        <SettingsView />
      {/if}
    </div>
  </div>
</div>
