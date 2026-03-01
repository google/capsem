<script lang="ts">
  import SubMenu from '../components/SubMenu.svelte';
  import { sidebarStore } from '../stores/sidebar.svelte';
  import type { SettingsSection } from '../types';
  import AiIcon from '../icons/AiIcon.svelte';
  import McpIcon from '../icons/McpIcon.svelte';
  import NetworkPolicyIcon from '../icons/NetworkPolicyIcon.svelte';
  import EnvironmentIcon from '../icons/EnvironmentIcon.svelte';
  import ResourcesIcon from '../icons/ResourcesIcon.svelte';
  import PaletteIcon from '../icons/PaletteIcon.svelte';
  import ProvidersSection from './settings/ProvidersSection.svelte';
  import McpSection from './settings/McpSection.svelte';
  import NetworkPolicySection from './settings/NetworkPolicySection.svelte';
  import EnvironmentSection from './settings/EnvironmentSection.svelte';
  import ResourcesSection from './settings/ResourcesSection.svelte';
  import AppearanceSection from './settings/AppearanceSection.svelte';

  const groups = [
    {
      label: 'AI',
      items: [{ id: 'providers', label: 'AI', icon: AiIcon }],
    },
    {
      label: 'MCP',
      items: [{ id: 'mcp-servers', label: 'MCP Servers', icon: McpIcon }],
    },
    {
      label: 'Network',
      items: [{ id: 'network-policy', label: 'Network Policy', icon: NetworkPolicyIcon }],
    },
    {
      label: 'VM',
      items: [
        { id: 'environment', label: 'Guest Environment', icon: EnvironmentIcon },
        { id: 'resources', label: 'Resources', icon: ResourcesIcon },
      ],
    },
    {
      label: 'App',
      items: [{ id: 'appearance', label: 'Appearance', icon: PaletteIcon }],
    },
  ];

  function onSelect(id: string) {
    sidebarStore.setSettingsSection(id as SettingsSection);
  }
</script>

<div class="flex h-full overflow-hidden">
  <SubMenu {groups} active={sidebarStore.settingsSection} {onSelect} />
  <div class="flex-1 overflow-auto p-4">
    {#if sidebarStore.settingsSection === 'providers'}
      <ProvidersSection />
    {:else if sidebarStore.settingsSection === 'mcp-servers'}
      <McpSection />
    {:else if sidebarStore.settingsSection === 'network-policy'}
      <NetworkPolicySection />
    {:else if sidebarStore.settingsSection === 'environment'}
      <EnvironmentSection />
    {:else if sidebarStore.settingsSection === 'resources'}
      <ResourcesSection />
    {:else if sidebarStore.settingsSection === 'appearance'}
      <AppearanceSection />
    {/if}
  </div>
</div>
