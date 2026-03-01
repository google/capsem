<script lang="ts">
  import type { TraceDetail, TraceModelCall, ToolCallEntry, ToolResponseEntry } from '../types';

  export type FlatItem = {
    call: TraceModelCall;
    callIndex: number;
  } & (
    | { type: 'thinking' }
    | { type: 'output' }
    | { type: 'tool'; tool: ToolCallEntry; response: ToolResponseEntry | null }
  );

  let { detail, onSelectItem, selectedItem = null, findToolResponse }: {
    detail: TraceDetail;
    onSelectItem?: (item: FlatItem) => void;
    selectedItem?: FlatItem | null;
    findToolResponse: (callId: string, genIndex: number, detail: TraceDetail) => ToolResponseEntry | null;
  } = $props();

  function formatTokens(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return `${n}`;
  }

  function formatCost(usd: number): string {
    if (usd === 0) return '$0';
    if (usd < 0.01) return `$${usd.toFixed(4)}`;
    return `$${usd.toFixed(2)}`;
  }

  const items = $derived.by(() => {
    const flat: FlatItem[] = [];
    for (let i = 0; i < detail.calls.length; i++) {
      const call = detail.calls[i];
      if (call.thinking_content) {
        flat.push({ type: 'thinking', call, callIndex: i });
      }
      if (call.text_content) {
        flat.push({ type: 'output', call, callIndex: i });
      }
      for (const tc of call.tool_calls) {
        const resp = findToolResponse(tc.call_id, i, detail);
        flat.push({ type: 'tool', tool: tc, response: resp, call, callIndex: i });
      }
    }
    return flat;
  });

  // Track which call IDs have already shown their metrics to avoid double-counting.
  const firstItemPerCall = $derived.by(() => {
    const seen = new Set<number>();
    const result = new Set<number>();
    for (let i = 0; i < items.length; i++) {
      const callId = items[i].call.id;
      if (!seen.has(callId)) {
        seen.add(callId);
        result.add(i);
      }
    }
    return result;
  });

  function isSelected(item: FlatItem): boolean {
    if (!selectedItem) return false;
    if (item.type !== selectedItem.type || item.call.id !== selectedItem.call.id) return false;
    if (item.type === 'tool' && selectedItem.type === 'tool') return item.tool.call_id === selectedItem.tool.call_id;
    return true;
  }
</script>

<div class="rounded-lg border border-base-300 overflow-hidden">
  <table class="table table-zebra table-xs w-full">
    <thead class="bg-base-200">
      <tr>
        <th>Type</th>
        <th class="text-right">In</th>
        <th class="text-right">Out</th>
        <th class="text-right">Cost</th>
        <th class="text-right">Time</th>
      </tr>
    </thead>
    <tbody>
      {#each items as item, idx}
        {@const isFirst = firstItemPerCall.has(idx)}
        <tr
          class="cursor-pointer hover:bg-base-200/80 {isSelected(item) ? 'bg-info/10' : ''}"
          onclick={() => onSelectItem?.(item)}
        >
          <td>
            {#if item.type === 'thinking'}
              <span class="badge badge-xs badge-ghost italic">thinking</span>
            {:else if item.type === 'output'}
              <span class="badge badge-xs badge-info">output</span>
            {:else}
              <span class="badge badge-xs badge-info font-mono">{item.tool.tool_name}</span>
              {#if item.response?.is_error}
                <span class="badge badge-xs badge-secondary ml-1">err</span>
              {/if}
            {/if}
          </td>
          {#if isFirst}
            <td class="text-right tabular-nums">{item.call.input_tokens != null ? formatTokens(item.call.input_tokens) : '-'}{#if item.call.usage_details?.cache_read}<span class="text-base-content/30 ml-0.5">({formatTokens(item.call.usage_details.cache_read)} cached)</span>{/if}</td>
            <td class="text-right tabular-nums">{item.call.output_tokens != null ? formatTokens(item.call.output_tokens) : '-'}</td>
            <td class="text-right tabular-nums text-info">{formatCost(item.call.estimated_cost_usd)}</td>
            <td class="text-right tabular-nums">{item.call.duration_ms}ms</td>
          {:else}
            <td class="text-right tabular-nums text-base-content/20">-</td>
            <td class="text-right tabular-nums text-base-content/20">-</td>
            <td class="text-right tabular-nums text-base-content/20">-</td>
            <td class="text-right tabular-nums text-base-content/20">-</td>
          {/if}
        </tr>
      {/each}
    </tbody>
  </table>
</div>
