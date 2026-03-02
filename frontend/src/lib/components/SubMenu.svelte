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
      <div class="my-2 border-t border-base-300"></div>
    {/if}
    {#if group.items.length > 1}
      <div class="px-2 mb-1">
        <span class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider">{group.label}</span>
      </div>
    {/if}
    <ul class="flex flex-col gap-0.5">
      {#each group.items as item}
        {@const isActive = active === item.id}
        <li>
          <button
            class="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-xs transition-colors {isActive
              ? 'bg-primary/15 text-primary font-medium'
              : 'text-base-content/60 hover:bg-base-300 hover:text-base-content'}"
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
