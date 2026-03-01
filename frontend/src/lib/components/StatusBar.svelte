<script lang="ts">
  import VmStateIndicator from './VmStateIndicator.svelte';
  import AnalyticsIcon from '../icons/AnalyticsIcon.svelte';
  import { networkStore } from '../stores/network.svelte';
  import { sidebarStore } from '../stores/sidebar.svelte';
  import { vmStore } from '../stores/vm.svelte';

  const isAnalyticsView = $derived(sidebarStore.activeView === 'analytics');
</script>

<footer class="flex flex-shrink-0 items-center justify-between border-t border-base-300 bg-base-200 px-3 py-1 text-xs text-base-content/60">
  <div class="flex items-center gap-2">
    <VmStateIndicator />
    {#if vmStore.terminalRenderer}
      <span class="text-base-content/40">{vmStore.terminalRenderer === 'webgl' ? 'WebGL' : 'Canvas'}</span>
    {/if}
  </div>
  <span>HTTPS: {networkStore.allowedCount} ok / {networkStore.deniedCount} denied</span>
  <button
    class="flex items-center gap-1 rounded px-1.5 py-0.5 transition-colors {isAnalyticsView
      ? 'bg-primary/15 text-primary'
      : 'hover:bg-base-300 hover:text-base-content'}"
    onclick={() => sidebarStore.setView(isAnalyticsView ? 'terminal' : 'analytics')}
    title="Analytics"
  >
    <AnalyticsIcon class="size-4" />
    <span>Analytics</span>
  </button>
</footer>
