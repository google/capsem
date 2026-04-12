<script lang="ts">
  import { onMount } from 'svelte';
  import TabBar from './TabBar.svelte';
  import Toolbar from './Toolbar.svelte';
  import NewTabPage from './NewTabPage.svelte';
  import VMFrame from './VMFrame.svelte';
  import SettingsPage from './SettingsPage.svelte';
  import StatsView from '../views/StatsView.svelte';
  import LogsView from '../views/LogsView.svelte';
  import ServiceLogsView from '../views/ServiceLogsView.svelte';
  import FilesView from '../views/FilesView.svelte';
  import InspectorView from '../views/InspectorView.svelte';
  import { tabStore } from '../../stores/tabs.svelte.ts';
  import { gatewayStore } from '../../stores/gateway.svelte.ts';
  import { vmStore } from '../../stores/vms.svelte.ts';

  let active = $derived(tabStore.active);
  const vmViews = ['terminal', 'stats', 'logs', 'files', 'inspector'] as const;
  let isVmView = $derived(active != null && active.vmId != null && vmViews.includes(active.view as any));

  onMount(() => {
    gatewayStore.init();
    vmStore.startPolling();
    return () => {
      vmStore.destroy();
      gatewayStore.destroy();
    };
  });
</script>

<div class="flex flex-col h-full">
  <TabBar />
  <Toolbar />

  <div class="flex-1 overflow-hidden bg-background">
    {#if active}
      {#if active.view === 'new-tab'}
        <NewTabPage />
      {:else if isVmView && active.vmId}
        <div class="h-full relative">
          <div class="absolute inset-0" class:hidden={active.view !== 'terminal'}>
            <VMFrame vmId={active.vmId} tabId={active.id} />
          </div>
          {#if active.view === 'stats'}
            <div class="absolute inset-0"><StatsView vmId={active.vmId} /></div>
          {:else if active.view === 'logs'}
            <div class="absolute inset-0"><LogsView vmId={active.vmId} /></div>
          {:else if active.view === 'files'}
            <div class="absolute inset-0"><FilesView vmId={active.vmId} /></div>
          {:else if active.view === 'inspector'}
            <div class="absolute inset-0"><InspectorView vmId={active.vmId} /></div>
          {/if}
        </div>
      {:else if active.view === 'logs' && !active.vmId}
        <ServiceLogsView />
      {:else if active.view === 'settings'}
        <SettingsPage />
      {:else}
        <div class="flex h-full items-center justify-center">
          <p class="text-muted-foreground-1">View: {active.view}</p>
        </div>
      {/if}
    {/if}
  </div>
</div>
