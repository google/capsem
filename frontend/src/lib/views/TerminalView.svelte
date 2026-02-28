<script lang="ts">
  import Terminal from '../components/Terminal.svelte';
  import { networkStore } from '../stores/network.svelte';
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

  const stats = $derived(networkStore.stats?.stats);
</script>

<div class="flex h-full flex-col">
  <div class="flex-1 min-h-0">
    <Terminal />
  </div>
  <div class="flex items-center justify-end gap-3 px-3 py-1 text-[11px] tabular-nums" style:background-color={termBg} style:color={termFg}>
    {#if stats && stats.model_call_count > 0}
      <span>{formatTokens(stats.total_input_tokens + stats.total_output_tokens)} tokens</span>
      <span>{stats.total_tool_calls} tools</span>
      <span>{formatCost(stats.total_estimated_cost_usd)}</span>
    {/if}
    <span class="flex items-center gap-1.5">
      <span>{vmStore.vmState}</span>
      <span class="inline-block size-1.5 rounded-full" style:background-color={vmStore.vmState === 'running' ? '#4ade80' : vmStore.vmState === 'booting' ? '#facc15' : '#ef4444'}></span>
    </span>
  </div>
</div>
