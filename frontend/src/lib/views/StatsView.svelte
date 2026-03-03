<script lang="ts">
  import SubMenu from '../components/SubMenu.svelte';
  import { statsStore } from '../stores/stats.svelte';
  import type { StatsTab } from '../types';
  import ModelsTab from './stats/ModelsTab.svelte';
  import ToolsTab from './stats/ToolsTab.svelte';
  import NetworkTab from './stats/NetworkTab.svelte';
  import FilesTab from './stats/FilesTab.svelte';

  const groups = [
    {
      label: '',
      items: [
        { id: 'models', label: 'Models' },
        { id: 'tools', label: 'Tools' },
        { id: 'network', label: 'Network' },
        { id: 'files', label: 'Files' },
      ],
    },
  ];

  function onSelect(id: string) {
    statsStore.setTab(id as StatsTab);
  }
</script>

<div class="flex h-full overflow-hidden">
  <SubMenu {groups} active={statsStore.activeTab} {onSelect} />
  <div class="flex-1 min-h-0 overflow-hidden">
    {#if statsStore.activeTab === 'models'}
      <ModelsTab />
    {:else if statsStore.activeTab === 'tools'}
      <ToolsTab />
    {:else if statsStore.activeTab === 'network'}
      <NetworkTab />
    {:else if statsStore.activeTab === 'files'}
      <FilesTab />
    {/if}
  </div>
</div>
