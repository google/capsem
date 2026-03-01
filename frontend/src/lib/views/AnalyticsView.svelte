<script lang="ts">
  import SubMenu from '../components/SubMenu.svelte';
  import { sidebarStore } from '../stores/sidebar.svelte';
  import type { AnalyticsSection } from '../types';
  import DashboardIcon from '../icons/DashboardIcon.svelte';
  import AiIcon from '../icons/AiIcon.svelte';
  import McpIcon from '../icons/McpIcon.svelte';
  import TrafficIcon from '../icons/TrafficIcon.svelte';
  import FilesIcon from '../icons/FilesIcon.svelte';
  import DashboardSection from './analytics/DashboardSection.svelte';
  import ModelsSection from './analytics/ModelsSection.svelte';
  import McpSection from './analytics/McpSection.svelte';
  import TrafficSection from './analytics/TrafficSection.svelte';
  import FilesSection from './analytics/FilesSection.svelte';

  const groups = [
    {
      label: 'Overview',
      items: [{ id: 'dashboard', label: 'Dashboard', icon: DashboardIcon }],
    },
    {
      label: 'Models',
      items: [{ id: 'models', label: 'AI', icon: AiIcon }],
    },
    {
      label: 'MCP',
      items: [{ id: 'mcp', label: 'MCP', icon: McpIcon }],
    },
    {
      label: 'Network',
      items: [{ id: 'traffic', label: 'Network', icon: TrafficIcon }],
    },
    {
      label: 'Workspace',
      items: [{ id: 'files', label: 'Files', icon: FilesIcon }],
    },
  ];

  function onSelect(id: string) {
    sidebarStore.setAnalyticsSection(id as AnalyticsSection);
  }
</script>

<div class="flex h-full overflow-hidden">
  <SubMenu {groups} active={sidebarStore.analyticsSection} {onSelect} />
  <div class="flex-1 overflow-auto p-4">
    {#if sidebarStore.analyticsSection === 'dashboard'}
      <DashboardSection />
    {:else if sidebarStore.analyticsSection === 'models'}
      <ModelsSection />
    {:else if sidebarStore.analyticsSection === 'mcp'}
      <McpSection />
    {:else if sidebarStore.analyticsSection === 'traffic'}
      <TrafficSection />
    {:else if sidebarStore.analyticsSection === 'files'}
      <FilesSection />
    {/if}
  </div>
</div>
