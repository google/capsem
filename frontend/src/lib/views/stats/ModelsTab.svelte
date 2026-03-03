<script lang="ts">
  import { onMount } from 'svelte';
  import { queryDb, queryAll } from '../../db';
  import { TRACES_SQL, TRACE_DETAIL_SQL, TRACE_TOOL_CALLS_SQL, TRACE_TOOL_RESPONSES_SQL } from '../../sql';
  import type { TraceSummary, TraceModelCall, ToolCallEntry, ToolResponseEntry, DetailSelection } from '../../types';
  import DetailPanel from './DetailPanel.svelte';

  let traces = $state<TraceSummary[]>([]);
  let expandedTraces = $state<Set<string>>(new Set());
  let traceDetails = $state<Map<string, TraceModelCall[]>>(new Map());
  let traceToolCalls = $state<Map<string, ToolCallEntry[]>>(new Map());
  let traceToolResponses = $state<Map<string, ToolResponseEntry[]>>(new Map());
  let detail = $state<DetailSelection | null>(null);
  let loaded = $state(false);

  onMount(async () => {
    await loadTraces();
    loaded = true;
  });

  async function loadTraces() {
    try {
      const res = await queryDb(TRACES_SQL, [50]);
      traces = queryAll<TraceSummary>(res);
    } catch (e) {
      console.error('Failed to load traces:', e);
    }
  }

  async function toggleTrace(traceId: string) {
    const next = new Set(expandedTraces);
    if (next.has(traceId)) {
      next.delete(traceId);
      expandedTraces = next;
      return;
    }
    next.add(traceId);
    expandedTraces = next;

    // Lazy-load detail if not cached.
    if (!traceDetails.has(traceId)) {
      try {
        const [detailRes, tcRes, trRes] = await Promise.all([
          queryDb(TRACE_DETAIL_SQL, [traceId]),
          queryDb(TRACE_TOOL_CALLS_SQL, [traceId]),
          queryDb(TRACE_TOOL_RESPONSES_SQL, [traceId]),
        ]);
        const calls = queryAll<TraceModelCall>(detailRes);
        const toolCalls = queryAll<ToolCallEntry>(tcRes);
        const toolResps = queryAll<ToolResponseEntry>(trRes);

        traceDetails = new Map(traceDetails).set(traceId, calls);
        traceToolCalls = new Map(traceToolCalls).set(traceId, toolCalls);
        traceToolResponses = new Map(traceToolResponses).set(traceId, toolResps);
      } catch (e) {
        console.error('Failed to load trace detail:', e);
      }
    }
  }

  function fmtTokens(n: number): string {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M';
    if (n >= 1_000) return (n / 1_000).toFixed(1) + 'K';
    return String(n);
  }

  function fmtDuration(ms: number): string {
    if (ms >= 60_000) return (ms / 60_000).toFixed(1) + 'm';
    if (ms >= 1_000) return (ms / 1_000).toFixed(1) + 's';
    return ms + 'ms';
  }

  function providerColor(provider: string | null): string {
    switch (provider) {
      case 'anthropic': return 'text-provider-anthropic';
      case 'google': return 'text-provider-google';
      case 'openai': return 'text-provider-openai';
      case 'mistral': return 'text-provider-mistral';
      default: return 'text-provider-fallback';
    }
  }

  function truncate(text: string | null, len: number): string {
    if (!text) return '';
    if (text.length <= len) return text;
    return text.substring(0, len) + '...';
  }

  function selectThinking(call: TraceModelCall) {
    detail = { type: 'thinking', data: { thinking_content: call.thinking_content } };
  }

  function selectText(call: TraceModelCall) {
    detail = { type: 'text', data: { text_content: call.text_content } };
  }

  function selectTool(tc: ToolCallEntry, traceId: string) {
    const responses = traceToolResponses.get(traceId) ?? [];
    const resp = responses.find(r => r.call_id === tc.call_id);
    detail = {
      type: 'tool',
      data: {
        tool_name: tc.tool_name,
        arguments: tc.arguments,
        origin: tc.origin,
        content_preview: resp?.content_preview ?? undefined,
        is_error: resp?.is_error ?? 0,
      },
    };
  }

  /** Get tool calls for a specific model_call within a trace. */
  function callToolCalls(traceId: string, modelCallId: number): ToolCallEntry[] {
    const all = traceToolCalls.get(traceId) ?? [];
    return all.filter(tc => tc.model_call_id === modelCallId);
  }
</script>

<div class="flex h-full overflow-hidden">
  <!-- Left panel: trace list -->
  <div class="flex-1 min-w-0 overflow-auto">
    {#if !loaded}
      <div class="flex items-center justify-center h-32">
        <span class="loading loading-spinner loading-md"></span>
      </div>
    {:else if traces.length === 0}
      <div class="flex items-center justify-center h-32 text-base-content/40 text-sm">
        No traces recorded yet.
      </div>
    {:else}
      <table class="w-full text-xs">
        <thead>
          <tr class="text-base-content/40 border-b border-base-200">
            <th class="pl-3 pr-1 py-1.5 w-5"></th>
            <th class="py-1.5 pr-2 text-left font-medium">Provider</th>
            <th class="py-1.5 pr-2 text-left font-medium">Model</th>
            <th class="py-1.5 pr-2 text-right font-medium">In</th>
            <th class="py-1.5 pr-2 text-right font-medium">Out</th>
            <th class="py-1.5 pr-2 text-right font-medium">Tools</th>
            <th class="py-1.5 pr-2 text-right font-medium">Time</th>
            <th class="py-1.5 pr-3 text-right font-medium">Cost</th>
          </tr>
        </thead>
        <tbody>
          {#each traces as trace}
            {@const isExpanded = expandedTraces.has(trace.trace_id)}
            {@const calls = traceDetails.get(trace.trace_id) ?? []}

            <!-- Trace header row -->
            <tr
              class="hover:bg-base-200/40 cursor-pointer transition-colors border-b border-base-200"
              onclick={() => toggleTrace(trace.trace_id)}
            >
              <td class="pl-3 pr-1 py-2 w-5">
                <svg class="size-3 transition-transform {isExpanded ? 'rotate-90' : ''}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="9 18 15 12 9 6"/></svg>
              </td>
              <td class="py-2 pr-2 whitespace-nowrap font-medium {providerColor(trace.provider)}">{trace.provider ?? 'unknown'}</td>
              <td class="py-2 pr-2 font-mono text-base-content/70">{trace.model ?? '?'}</td>
              <td class="py-2 pr-2 whitespace-nowrap text-base-content/50 text-right">{fmtTokens(trace.total_input_tokens)} in</td>
              <td class="py-2 pr-2 whitespace-nowrap text-base-content/50 text-right">{fmtTokens(trace.total_output_tokens)} out</td>
              <td class="py-2 pr-2 whitespace-nowrap text-base-content/40 text-right">{trace.total_tool_calls > 0 ? `${trace.total_tool_calls} tools` : ''}</td>
              <td class="py-2 pr-2 whitespace-nowrap text-base-content/40 font-mono text-right">{trace.total_duration_ms ? fmtDuration(trace.total_duration_ms) : ''}</td>
              <td class="py-2 pr-3 whitespace-nowrap text-base-content/40 font-mono text-right">${trace.total_cost.toFixed(3)}</td>
            </tr>

            <!-- Expanded: flat span rows -->
            {#if isExpanded}
              {#each calls as call}
                {@const toolCalls = callToolCalls(trace.trace_id, call.id)}
                {#if call.thinking_content}
                  <tr
                    class="bg-base-200/20 hover:bg-base-300/30 cursor-pointer transition-colors"
                    onclick={() => selectThinking(call)}
                  >
                    <td></td>
                    <td class="py-1 pr-2">
                      <span class="badge badge-xs w-10 text-center bg-span-thinking/15 text-span-thinking border-0">think</span>
                    </td>
                    <td class="py-1 text-base-content/50 truncate max-w-0" colspan="6">{truncate(call.thinking_content, 100)}</td>
                  </tr>
                {/if}
                {#each toolCalls as tc}
                  <tr
                    class="bg-base-200/20 hover:bg-base-300/30 cursor-pointer transition-colors"
                    onclick={() => selectTool(tc, trace.trace_id)}
                  >
                    <td></td>
                    <td class="py-1 pr-2">
                      <span class="badge badge-xs w-10 text-center bg-span-tool/15 text-span-tool border-0">tool</span>
                    </td>
                    <td class="py-1 font-mono text-base-content/70 whitespace-nowrap pr-2">{tc.tool_name}</td>
                    <td class="py-1 text-base-content/30 truncate max-w-0" colspan="5">{truncate(tc.arguments, 60)}</td>
                  </tr>
                {/each}
                {#if call.text_content}
                  <tr
                    class="bg-base-200/20 hover:bg-base-300/30 cursor-pointer transition-colors"
                    onclick={() => selectText(call)}
                  >
                    <td></td>
                    <td class="py-1 pr-2">
                      <span class="badge badge-xs w-10 text-center bg-span-answer/15 text-span-answer border-0">text</span>
                    </td>
                    <td class="py-1 text-base-content/50 truncate max-w-0" colspan="6">{truncate(call.text_content, 100)}</td>
                  </tr>
                {/if}
                {#if !call.thinking_content && !call.text_content && callToolCalls(trace.trace_id, call.id).length === 0}
                  <tr class="bg-base-200/20">
                    <td></td>
                    <td class="py-1 pr-2"></td>
                    <td class="py-1 text-base-content/30 italic" colspan="6">no content captured</td>
                  </tr>
                {/if}
              {/each}
            {/if}
          {/each}
        </tbody>
      </table>
    {/if}
  </div>

  <!-- Right detail panel -->
  {#if detail}
    <DetailPanel selection={detail} onClose={() => { detail = null; }} />
  {/if}
</div>
