<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { statsStore } from '../stores/stats.svelte';
  import { sidebarStore } from '../stores/sidebar.svelte';

  onMount(() => statsStore.start());
  onDestroy(() => statsStore.stop());

  function openStats() {
    sidebarStore.setView('stats');
  }
</script>

<div class="flex items-center gap-4 px-3 py-1 bg-black text-xs text-base-content/60 select-none">
  <span class="font-mono">{statsStore.totalTokens} tokens</span>
  <span class="text-base-content/20">|</span>
  <span class="font-mono">{statsStore.toolCount} tools</span>
  <span class="text-base-content/20">|</span>
  <span class="font-mono">{statsStore.totalCost}</span>
  <span class="flex-1"></span>
  <button
    class="text-xs text-base-content/50 hover:text-interactive transition-colors"
    onclick={openStats}
  >
    Stats &rsaquo;
  </button>
</div>
