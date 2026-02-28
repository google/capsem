<script lang="ts">
  import { sidebarStore } from '../stores/sidebar.svelte';
  import type { ViewName } from '../types';
  import type { Component } from 'svelte';
  import TerminalIcon from '../icons/TerminalIcon.svelte';
  import SessionsIcon from '../icons/SessionsIcon.svelte';
  import SettingsIcon from '../icons/SettingsIcon.svelte';
  import ThemeToggle from './ThemeToggle.svelte';

  const items: { view: ViewName; label: string; icon: Component }[] = [
    { view: 'terminal', label: 'Console', icon: TerminalIcon },
    { view: 'sessions', label: 'Sessions', icon: SessionsIcon },
  ];

  const isSettings = $derived(sidebarStore.activeView === 'settings');
</script>

<aside
  class="flex flex-col flex-shrink-0 border-r border-base-300 bg-base-200 transition-[width] duration-150 overflow-hidden"
  style:width={sidebarStore.collapsed ? '48px' : '192px'}
>
  <nav class="flex-1 py-2">
    <ul class="flex flex-col gap-1 px-1.5">
      {#each items as item}
        {@const isActive = sidebarStore.activeView === item.view}
        <li>
          <button
            class="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-sm transition-colors {isActive
              ? 'bg-primary/15 text-primary'
              : 'text-base-content/60 hover:bg-base-300 hover:text-base-content'}"
            onclick={() => sidebarStore.setView(item.view)}
            title={sidebarStore.collapsed ? item.label : undefined}
          >
            <svelte:component this={item.icon} />
            {#if !sidebarStore.collapsed}
              <span class="text-xs font-medium whitespace-nowrap">{item.label}</span>
            {/if}
          </button>
        </li>
      {/each}
    </ul>
  </nav>
  <div class="flex flex-col items-center gap-1 border-t border-base-300 py-2 px-1.5">
    <ThemeToggle />
    <button
      class="btn btn-ghost btn-xs {isSettings ? 'text-primary' : ''}"
      onclick={() => sidebarStore.setView(isSettings ? 'terminal' : 'settings')}
      title="Settings"
    >
      <svelte:component this={SettingsIcon} />
    </button>
    <button
      class="btn btn-ghost btn-xs"
      onclick={() => sidebarStore.toggleCollapsed()}
      title={sidebarStore.collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        class="size-4 transition-transform {sidebarStore.collapsed ? '' : 'rotate-180'}"
      >
        <path d="m9 18 6-6-6-6" />
      </svg>
    </button>
  </div>
</aside>
