<script lang="ts">
  import SubMenu from '../components/SubMenu.svelte';
  import { statsStore } from '../stores/stats.svelte';
  import type { StatsTab } from '../types';
  import AITab from './stats/AITab.svelte';
  import ToolsTab from './stats/ToolsTab.svelte';
  import NetworkTab from './stats/NetworkTab.svelte';
  import FilesTab from './stats/FilesTab.svelte';
  import SnapshotsTab from './stats/SnapshotsTab.svelte';

  const groups = [
    {
      label: '',
      items: [
        { id: 'ai', label: 'Model' },
        { id: 'tools', label: 'Tools' },
        { id: 'network', label: 'Network' },
        { id: 'files', label: 'Files' },
        { id: 'snapshots', label: 'Snapshots' },
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
    {#if statsStore.activeTab === 'ai'}
      <AITab />
    {:else if statsStore.activeTab === 'tools'}
      <ToolsTab />
    {:else if statsStore.activeTab === 'network'}
      <NetworkTab />
    {:else if statsStore.activeTab === 'files'}
      <FilesTab />
    {:else if statsStore.activeTab === 'snapshots'}
      <SnapshotsTab />
    {/if}
  </div>
</div>
