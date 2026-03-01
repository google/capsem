<script lang="ts">
  import { sidebarStore } from '../stores/sidebar.svelte';
  import type { ViewName } from '../types';
  import type { Component } from 'svelte';
  import TerminalIcon from '../icons/TerminalIcon.svelte';
  import AnalyticsIcon from '../icons/AnalyticsIcon.svelte';
  import SettingsIcon from '../icons/SettingsIcon.svelte';

  const items: { view: ViewName; label: string; icon: Component<{ class?: string }> }[] = [
    { view: 'terminal', label: 'Console', icon: TerminalIcon },
    { view: 'analytics', label: 'Analytics', icon: AnalyticsIcon },
    { view: 'settings', label: 'Settings', icon: SettingsIcon },
  ];
</script>

<aside
  class="flex flex-col flex-shrink-0 border-r border-base-300 bg-base-200 w-12 overflow-hidden"
>
  <nav class="flex-1 py-2">
    <ul class="flex flex-col gap-1 px-1.5">
      {#each items as item}
        {@const isActive = sidebarStore.activeView === item.view}
        {@const Icon = item.icon}
        <li>
          <button
            class="flex w-full items-center justify-center rounded-lg px-2 py-2 transition-colors {isActive
              ? 'bg-primary/15 text-primary'
              : 'text-base-content/60 hover:bg-base-300 hover:text-base-content'}"
            onclick={() => sidebarStore.setView(item.view)}
            title={item.label}
          >
            <Icon class="size-5" />
          </button>
        </li>
      {/each}
    </ul>
  </nav>
</aside>
