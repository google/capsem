<script lang="ts">
  import SubMenu from '../components/SubMenu.svelte';
  import { sidebarStore } from '../stores/sidebar.svelte';
  import { settingsStore } from '../stores/settings.svelte';
  import SettingsSection from './settings/SettingsSection.svelte';

  // Derive nav groups from the settings tree (one group per top-level node).
  const groups = $derived(
    settingsStore.sections.map((name) => ({
      label: name,
      items: [{ id: name, label: name }],
    })),
  );

  // Auto-select first section if nothing selected yet.
  $effect(() => {
    if (!sidebarStore.settingsSection && settingsStore.sections.length > 0) {
      sidebarStore.setSettingsSection(settingsStore.sections[0]);
    }
  });

  const activeGroup = $derived(settingsStore.section(sidebarStore.settingsSection));

  function onSelect(id: string) {
    sidebarStore.setSettingsSection(id);
  }
</script>

<div class="flex h-full overflow-hidden">
  <SubMenu {groups} active={sidebarStore.settingsSection} {onSelect} />
  <div class="flex-1 overflow-auto p-4">
    {#if activeGroup && activeGroup.kind === 'group'}
      <SettingsSection group={activeGroup} />
    {:else if settingsStore.loading}
      <div class="flex items-center justify-center h-full">
        <span class="loading loading-spinner loading-md"></span>
      </div>
    {:else}
      <p class="text-base-content/40 text-sm">Select a section from the left.</p>
    {/if}
  </div>
</div>
