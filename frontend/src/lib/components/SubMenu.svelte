<script lang="ts">
  import type { Component } from 'svelte';

  interface SubMenuItem {
    id: string;
    label: string;
    icon?: Component<{ class?: string }>;
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
</script>

<aside class="flex-shrink-0 w-[200px] border-r border-base-300 bg-base-200/50 overflow-y-auto py-3 px-2">
  {#each groups as group, gi}
    {#if gi > 0}
      <div class="divider my-1"></div>
    {/if}
    <ul class="menu menu-sm p-0">
      {#if group.items.length > 1 && group.label}
        <li class="menu-title text-[10px] uppercase tracking-wider">{group.label}</li>
      {/if}
      {#each group.items as item}
        {@const isActive = active === item.id}
        <li>
          <button
            class="text-xs {isActive ? 'menu-active' : ''}"
            onclick={() => onSelect(item.id)}
          >
            {#if item.icon}
              {@const Icon = item.icon}
              <Icon class="size-4" />
            {/if}
            <span class="whitespace-nowrap">{item.label}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/each}
</aside>
