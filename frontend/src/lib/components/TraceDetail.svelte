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
      {#each items as item}
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
          <td class="text-right tabular-nums">{item.call.input_tokens != null ? formatTokens(item.call.input_tokens) : '-'}</td>
          <td class="text-right tabular-nums">{item.call.output_tokens != null ? formatTokens(item.call.output_tokens) : '-'}</td>
          <td class="text-right tabular-nums text-info">{formatCost(item.call.estimated_cost_usd)}</td>
          <td class="text-right tabular-nums">{item.call.duration_ms}ms</td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>
