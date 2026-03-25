<script lang="ts">
  import { tick } from 'svelte';
  import SubMenu from '../components/SubMenu.svelte';
  import { sidebarStore } from '../stores/sidebar.svelte';
  import { settingsStore } from '../stores/settings.svelte';
  import { mcpStore } from '../stores/mcp.svelte';
  import SettingsSection from './settings/SettingsSection.svelte';
  import McpSection from './settings/McpSection.svelte';
  import type { SettingsGroup } from '../types';

  /** Check if a section has any child groups (for collapsible sidebar). */
  function hasChildGroups(node: SettingsGroup): boolean {
    return node.children.some((c) => c.kind === 'group');
  }

  /** Find the top-level parent section that contains a subgroup by name. */
  function findParentSection(name: string): SettingsGroup | undefined {
    for (const node of settingsStore.tree) {
      if (node.kind !== 'group') continue;
      for (const child of node.children) {
        if (child.kind === 'group' && child.name === name) return node;
      }
    }
    return undefined;
  }

  // Derive nav groups from the settings tree.
  // Sections with non-toggle child groups (like VM > Environment, VM > Resources)
  // get a collapsible parent with children in the sidebar.
  const MCP_GROUP_ID = 'MCP Servers';

  const groups = $derived(
    [
      ...settingsStore.tree
        .filter((n): n is SettingsGroup => n.kind === 'group')
        .map((section) => {
          if (hasChildGroups(section)) {
            const children = section.children
              .filter((c): c is SettingsGroup => c.kind === 'group')
              .map((c) => ({ id: c.name, label: c.name }));
            return {
              label: section.name,
              items: [{ id: section.name, label: section.name, children }],
            };
          }
          return {
            label: section.name,
            items: [{ id: section.name, label: section.name }],
          };
        }),
      {
        label: MCP_GROUP_ID,
        items: [{
          id: MCP_GROUP_ID,
          label: MCP_GROUP_ID,
          children: [
            { id: 'mcp-policy', label: 'Policy' },
            { id: 'mcp-local', label: 'Local Tools' },
            { id: 'mcp-servers', label: 'Servers' },
          ],
        }],
      },
    ],
  );

  // Auto-select first section if nothing selected yet.
  $effect(() => {
    if (!sidebarStore.settingsSection && settingsStore.sections.length > 0) {
      sidebarStore.setSettingsSection(settingsStore.sections[0]);
    }
  });

  /** Whether the current selection is in the MCP section. */
  const isMcpActive = $derived(
    sidebarStore.settingsSection === MCP_GROUP_ID ||
    sidebarStore.settingsSection === 'mcp-policy' ||
    sidebarStore.settingsSection === 'mcp-local' ||
    sidebarStore.settingsSection === 'mcp-servers',
  );

  // The active section is always a top-level group. Subgroup clicks resolve to their parent.
  const activeGroup = $derived.by(() => {
    if (isMcpActive) return undefined;
    const sel = sidebarStore.settingsSection;
    const topLevel = settingsStore.section(sel);
    if (topLevel) return topLevel;
    // Selected id is a subgroup name -- find its parent section
    const parent = findParentSection(sel);
    return parent ?? undefined;
  });

  // Suppress scroll-based highlight updates while a programmatic scroll is in progress.
  let suppressScrollHighlight = false;

  // Reference to the scroll container.
  let scrollContainer: HTMLDivElement | undefined = $state();

  /** On scroll, highlight the subgroup heading that best represents what the user is viewing.
   *  Uses a "last heading that scrolled past the top half" heuristic so sections that can't
   *  reach the very top (short content at end of page) still get highlighted. */
  function onScroll() {
    if (suppressScrollHighlight || !scrollContainer) return;
    const headings = scrollContainer.querySelectorAll<HTMLElement>('[data-subgroup]');
    if (headings.length === 0) return;
    const containerTop = scrollContainer.getBoundingClientRect().top;
    const containerHeight = scrollContainer.clientHeight;
    // Threshold: a heading in the top 40% of the container counts as "current".
    const threshold = containerHeight * 0.4;
    let best: HTMLElement | null = null;
    for (const el of headings) {
      const dist = el.getBoundingClientRect().top - containerTop;
      if (dist <= threshold) {
        // Among headings in the top portion, pick the LAST one (furthest scrolled).
        best = el;
      }
    }
    if (!best) best = headings[0];
    const name = best?.dataset.subgroup;
    if (name && name !== sidebarStore.settingsSection) {
      sidebarStore.setSettingsSection(name);
    }
  }

  function onSelect(id: string) {
    // MCP section handling
    if (id === MCP_GROUP_ID) {
      sidebarStore.setSettingsSection(id);
      if (scrollContainer) scrollContainer.scrollTop = 0;
      return;
    }
    if (id === 'mcp-policy' || id === 'mcp-local' || id === 'mcp-servers') {
      sidebarStore.setSettingsSection(id);
      suppressScrollHighlight = true;
      tick().then(() => {
        const el = scrollContainer?.querySelector<HTMLElement>(`[data-subgroup="${id}"]`);
        el?.scrollIntoView({ behavior: 'smooth', block: 'start' });
        setTimeout(() => { suppressScrollHighlight = false; }, 500);
      });
      return;
    }

    const isTopLevel = settingsStore.section(id) !== undefined;
    if (isTopLevel) {
      sidebarStore.setSettingsSection(id);
      // Reset scroll to top when switching sections.
      if (scrollContainer) scrollContainer.scrollTop = 0;
    } else {
      // It's a subgroup -- navigate to parent section and scroll to subgroup
      const parent = findParentSection(id);
      if (parent) {
        sidebarStore.setSettingsSection(id);
        suppressScrollHighlight = true;
        tick().then(() => {
          const el = document.getElementById(`settings-group-${id}`);
          el?.scrollIntoView({ behavior: 'smooth', block: 'start' });
          // Re-enable after scroll settles.
          setTimeout(() => { suppressScrollHighlight = false; }, 500);
        });
      }
    }
  }
</script>

<div class="flex h-full overflow-hidden">
  <SubMenu {groups} active={sidebarStore.settingsSection} {onSelect} />
  <div class="flex-1 overflow-auto p-4" bind:this={scrollContainer} onscroll={onScroll}>
    {#if isMcpActive}
      <McpSection />
    {:else if activeGroup && activeGroup.kind === 'group'}
      <SettingsSection group={activeGroup} />
    {:else if settingsStore.loading}
      <div class="flex items-center justify-center h-full">
        <span class="loading loading-spinner loading-md"></span>
      </div>
    {:else}
      <p class="text-base-content/40 text-sm">Select a section from the left.</p>
    {/if}
  </div>
  {#if settingsStore.isDirty}
    <div class="sticky bottom-0 bg-base-100 border-t border-base-300 px-4 py-2 flex items-center justify-end gap-2 z-10">
      <span class="text-xs text-base-content/50">{settingsStore.model?.pendingChanges.size ?? 0} unsaved change(s)</span>
      <button class="btn btn-ghost btn-sm" onclick={() => settingsStore.discard()}>Discard</button>
      <button class="btn btn-sm bg-interactive text-white" onclick={() => settingsStore.save()}>Save</button>
    </div>
  {/if}
</div>
