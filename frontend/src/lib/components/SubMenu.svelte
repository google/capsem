<script lang="ts">
  import type { Component } from 'svelte';

  /** A child menu entry (no further nesting). */
  interface SubMenuChild {
    id: string;
    label: string;
    icon?: Component<{ class?: string }>;
  }

  /** A menu item, optionally with one level of children. */
  interface SubMenuItem {
    id: string;
    label: string;
    icon?: Component<{ class?: string }>;
    children?: SubMenuChild[];
  }

  interface SubMenuGroup {
    label: string;
    items: SubMenuItem[];
  }

  let { groups, active, onSelect }: {
    groups: SubMenuGroup[];
    active: string;
    onSelect: (id: string) => void;
  } = $props();

  // Track which parent items are expanded.
  let expanded = $state<Set<string>>(new Set());

  /** Check if active id matches this item or any of its children. */
  function isActiveTree(item: SubMenuItem, activeId: string): boolean {
    if (item.id === activeId) return true;
    return item.children?.some((c) => c.id === activeId) ?? false;
  }

  // Auto-expand the parent whose tree contains the active item, collapse others.
  $effect(() => {
    const next = new Set<string>();
    for (const group of groups) {
      for (const item of group.items) {
        if (item.children && isActiveTree(item, active)) {
          next.add(item.id);
        }
      }
    }
    expanded = next;
  });

  function handleParentClick(item: SubMenuItem) {
    onSelect(item.id);
  }
</script>

<aside class="submenu flex-shrink-0 w-[200px] border-r border-base-300 bg-base-200/50 overflow-y-auto py-3 px-2">
  <ul class="menu p-0 gap-0.5">
    {#each groups as group}
      {#each group.items as item}
        {#if item.children && item.children.length > 0}
          {@const isParentActive = isActiveTree(item, active)}
          {@const isOpen = expanded.has(item.id) || isParentActive}
          <li>
            <button
              class="text-sm justify-between {isParentActive && active === item.id ? 'text-interactive font-semibold' : ''}"
              onclick={() => handleParentClick(item)}
            >
              <span class="flex items-center gap-2">
                {#if item.icon}
                  {@const Icon = item.icon}
                  <Icon class="size-4" />
                {/if}
                <span class="whitespace-nowrap">{item.label}</span>
              </span>
              <svg
                class="size-3 transition-transform duration-150 {isOpen ? 'rotate-180' : ''}"
                viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"
              ><polyline points="6 9 12 15 18 9" /></svg>
            </button>
            {#if isOpen}
              <ul class="ml-2 mt-0.5">
                {#each item.children as child}
                  {@const isChildActive = active === child.id}
                  <li>
                    <button
                      class="text-sm {isChildActive ? 'text-interactive font-semibold' : ''}"
                      onclick={() => onSelect(child.id)}
                    >
                      {#if child.icon}
                        {@const Icon = child.icon}
                        <Icon class="size-4" />
                      {/if}
                      <span class="whitespace-nowrap">{child.label}</span>
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          </li>
        {:else}
          {@const isActive = active === item.id}
          <li>
            <button
              class="text-sm {isActive ? 'text-interactive font-semibold' : ''}"
              onclick={() => onSelect(item.id)}
            >
              {#if item.icon}
                {@const Icon = item.icon}
                <Icon class="size-4" />
              {/if}
              <span class="whitespace-nowrap">{item.label}</span>
            </button>
          </li>
        {/if}
      {/each}
    {/each}
  </ul>
</aside>

<style>
  /* Override DaisyUI menu button backgrounds -- active state is purple text only. */
  .submenu :global(.menu li > button:focus),
  .submenu :global(.menu li > button:active),
  .submenu :global(.menu li > button.focus) {
    background-color: transparent;
  }
</style>
