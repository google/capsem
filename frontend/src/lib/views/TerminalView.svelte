<script lang="ts">
  import Terminal from '../components/Terminal.svelte';
  import { networkStore } from '../stores/network.svelte';
  import { sidebarStore } from '../stores/sidebar.svelte';
  import { themeStore } from '../stores/theme.svelte';
  import { vmStore } from '../stores/vm.svelte';

  const termBg = $derived(themeStore.theme === 'dark' ? '#000000' : '#f5f5f5');
  const termFg = $derived(themeStore.theme === 'dark' ? 'rgba(255,255,255,0.3)' : 'rgba(0,0,0,0.35)');

  function formatTokens(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return `${n}`;
  }

  function formatCost(usd: number): string {
    if (usd === 0) return '$0.00';
    if (usd < 0.01) return `$${usd.toFixed(4)}`;
    return `$${usd.toFixed(2)}`;
  }

  const mStats = $derived(networkStore.modelStats);
</script>

<div class="flex h-full flex-col">
  <div class="flex-1 min-h-0">
    <Terminal />
  </div>
  <div class="flex items-center justify-end gap-3 px-3 py-1 text-[11px] tabular-nums" style:background-color={termBg} style:color={termFg}>
    {#if mStats && mStats.model_call_count > 0}
      <span>{formatTokens(mStats.total_input_tokens + mStats.total_output_tokens)} tokens</span>
      <span>{networkStore.toolCount} tools</span>
      <span>{formatCost(mStats.total_estimated_cost_usd)}</span>
    {/if}
    <button
      class="flex items-center gap-1.5 rounded px-1.5 py-0.5 transition-colors hover:text-info/80 cursor-pointer"
      onclick={() => { sidebarStore.setAnalyticsSection('models'); sidebarStore.setView('analytics'); }}
      title="View session statistics"
    >
      <span>Session stats</span>
      <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="size-3">
        <polyline points="9 18 15 12 9 6"/>
      </svg>
    </button>
  </div>
</div>
