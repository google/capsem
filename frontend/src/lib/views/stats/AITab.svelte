<script lang="ts">
  import { onMount } from 'svelte';
  import { queryDb, queryAll } from '../../db';
  import {
    TRACES_SQL, TRACE_DETAIL_SQL, TRACE_TOOL_CALLS_SQL, TRACE_TOOL_RESPONSES_SQL,
    AI_TOKENS_OVER_TIME_BY_MODEL_SQL, AI_MODEL_USAGE_SQL,
  } from '../../sql';
  import { colors, modelColor } from '../../css-var';
  import { BarChart, PieChart } from 'layerchart';
  import type { TraceSummary, TraceModelCall, ToolCallEntry, ToolResponseEntry, DetailSelection } from '../../types';
  import StatCards from './StatCards.svelte';
  import DetailPanel from './DetailPanel.svelte';

  let traces = $state<TraceSummary[]>([]);
  let expandedTraces = $state<Set<string>>(new Set());
  let traceDetails = $state<Map<string, TraceModelCall[]>>(new Map());
  let traceToolCalls = $state<Map<string, ToolCallEntry[]>>(new Map());
  let traceToolResponses = $state<Map<string, ToolResponseEntry[]>>(new Map());
  let detail = $state<DetailSelection | null>(null);
  let loaded = $state(false);

  interface PieSeries { key: string; color: string; data: { key: string; value: number }[] }
  interface PieLegend { label: string; color: string; value: string }

  // Chart data
  let tokTimeData = $state<Record<string, unknown>[]>([]);
  let tokTimeSeries = $state<{ key: string; label: string; color: string }[]>([]);
  let tokenTypeSeries = $state<PieSeries[]>([]);
  let tokenTypeLegend = $state<PieLegend[]>([]);
  let tokenTypeTotal = $state('');
  let modelTokenSeries = $state<PieSeries[]>([]);
  let modelTokenLegend = $state<PieLegend[]>([]);
  let modelTokenTotal = $state('');
  let modelCostSeries = $state<PieSeries[]>([]);
  let modelCostLegend = $state<PieLegend[]>([]);
  let modelCostTotal = $state('');
  let statCards = $state<{ label: string; value: string | number; colorClass?: string }[]>([]);

  onMount(() => {
    Promise.all([loadTraces(), loadCharts()]).then(() => { loaded = true; });
  });

  async function loadTraces() {
    try {
      traces = queryAll<TraceSummary>(await queryDb(TRACES_SQL, [50]));
    } catch (e) { console.error('Failed to load traces:', e); }
  }

  async function loadCharts() {
    try {
      const [tokTimeRes, modelRes] = await Promise.all([
        queryDb(AI_TOKENS_OVER_TIME_BY_MODEL_SQL),
        queryDb(AI_MODEL_USAGE_SQL),
      ]);

      // Tokens over time -- one bar per call, stacked by model
      const tokRows = queryAll<{ bucket: number; model: string; provider: string; tokens: number }>(tokTimeRes);
      // Build model->provider map from raw rows (first occurrence wins)
      const modelProviderMap = new Map<string, string>();
      for (const r of tokRows) {
        if (!modelProviderMap.has(r.model)) modelProviderMap.set(r.model, r.provider);
      }
      const models = [...modelProviderMap.keys()];
      const buckets = [...new Set(tokRows.map(r => r.bucket))].sort((a, b) => a - b);
      // Register colors for all models (provider-aware) before building series
      for (const m of models) modelColor(m, modelProviderMap.get(m)!);
      tokTimeSeries = models.map(m => ({ key: m, label: m, color: modelColor(m, modelProviderMap.get(m)!) }));
      tokTimeData = buckets.map((b, i) => {
        const row: Record<string, unknown> = { bucket: String(i + 1) };
        for (const m of models) {
          row[m] = tokRows.find(r => r.bucket === b && r.model === m)?.tokens ?? 0;
        }
        return row;
      });

      // Model usage -- build pie series for all three charts
      const modelRows = queryAll<{ model: string; provider: string; input_tokens: number; output_tokens: number; tokens: number; cost: number; call_count: number }>(modelRes);
      for (const r of modelRows) modelColor(r.model, r.provider);

      // Token type pie (input vs output)
      const totalIn = modelRows.reduce((s, r) => s + r.input_tokens, 0);
      const totalOut = modelRows.reduce((s, r) => s + r.output_tokens, 0);
      const ttSeries: PieSeries[] = [];
      if (totalIn > 0) ttSeries.push({ key: 'Input', color: colors.tokenInput, data: [{ key: 'Input', value: totalIn }] });
      if (totalOut > 0) ttSeries.push({ key: 'Output', color: colors.tokenOutput, data: [{ key: 'Output', value: totalOut }] });
      tokenTypeSeries = ttSeries;
      tokenTypeLegend = [
        { label: 'Input', color: colors.tokenInput, value: fmtTokens(totalIn) },
        { label: 'Output', color: colors.tokenOutput, value: fmtTokens(totalOut) },
      ];
      tokenTypeTotal = fmtTokens(totalIn + totalOut);

      // Tokens per model pie
      modelTokenSeries = modelRows.filter(r => r.tokens > 0).map(r => ({
        key: r.model,
        color: modelColor(r.model, r.provider),
        data: [{ key: r.model, value: r.tokens }],
      }));
      modelTokenLegend = modelRows.filter(r => r.tokens > 0).map(r => ({
        label: r.model, color: modelColor(r.model, r.provider), value: fmtTokens(r.tokens),
      }));
      modelTokenTotal = fmtTokens(modelRows.reduce((s, r) => s + r.tokens, 0));

      // Cost per model pie
      const totalCostVal = modelRows.reduce((s, r) => s + r.cost, 0);
      modelCostSeries = modelRows.filter(r => r.cost > 0).map(r => ({
        key: r.model,
        color: modelColor(r.model, r.provider),
        data: [{ key: r.model, value: r.cost }],
      }));
      modelCostLegend = modelRows.filter(r => r.cost > 0).map(r => ({
        label: r.model, color: modelColor(r.model, r.provider), value: '$' + r.cost.toFixed(3),
      }));
      modelCostTotal = '$' + totalCostVal.toFixed(2);

      // Stat cards
      const callCount = modelRows.reduce((s, r) => s + r.call_count, 0);
      statCards = [
        { label: 'Calls', value: callCount },
        { label: 'Tokens In', value: fmtTokens(totalIn) },
        { label: 'Tokens Out', value: fmtTokens(totalOut) },
        { label: 'Cost', value: totalCostVal < 0.01 && totalCostVal > 0 ? '<$0.01' : '$' + totalCostVal.toFixed(2) },
      ];
    } catch (e) { console.error('AI charts failed:', e); }
  }

  async function toggleTrace(traceId: string) {
    const next = new Set(expandedTraces);
    if (next.has(traceId)) { next.delete(traceId); expandedTraces = next; return; }
    next.add(traceId);
    expandedTraces = next;
    if (!traceDetails.has(traceId)) {
      try {
        const [detailRes, tcRes, trRes] = await Promise.all([
          queryDb(TRACE_DETAIL_SQL, [traceId]),
          queryDb(TRACE_TOOL_CALLS_SQL, [traceId]),
          queryDb(TRACE_TOOL_RESPONSES_SQL, [traceId]),
        ]);
        traceDetails = new Map(traceDetails).set(traceId, queryAll<TraceModelCall>(detailRes));
        traceToolCalls = new Map(traceToolCalls).set(traceId, queryAll<ToolCallEntry>(tcRes));
        traceToolResponses = new Map(traceToolResponses).set(traceId, queryAll<ToolResponseEntry>(trRes));
      } catch (e) { console.error('Failed to load trace detail:', e); }
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

  function provColor(provider: string | null): string {
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
    return text.length <= len ? text : text.substring(0, len) + '...';
  }

  function selectThinking(call: TraceModelCall) { detail = { type: 'thinking', data: { thinking_content: call.thinking_content } }; }
  function selectText(call: TraceModelCall) { detail = { type: 'text', data: { text_content: call.text_content } }; }
  function selectInput(call: TraceModelCall) {
    detail = {
      type: 'model_input',
      data: {
        request_body_preview: call.request_body_preview,
        system_prompt_preview: call.system_prompt_preview,
        messages_count: call.messages_count,
        tools_count: call.tools_count,
      },
    };
  }
  function selectTool(tc: ToolCallEntry, traceId: string) {
    const resp = (traceToolResponses.get(traceId) ?? []).find(r => r.call_id === tc.call_id);
    detail = { type: 'tool', data: { tool_name: tc.tool_name, arguments: tc.arguments, origin: tc.origin, content_preview: resp?.content_preview ?? undefined, is_error: resp?.is_error ?? 0 } };
  }
  function callToolCalls(traceId: string, modelCallId: number): ToolCallEntry[] {
    return (traceToolCalls.get(traceId) ?? []).filter(tc => tc.model_call_id === modelCallId);
  }
</script>

<div class="flex h-full overflow-hidden">
  <div class="flex-1 min-w-0 flex flex-col overflow-hidden">
    {#if statCards.length > 0}
      <StatCards cards={statCards} />
    {/if}

    <div class="flex-1 min-h-0 overflow-auto">
    <!-- Charts: tokens over time bar + 3 pie charts -->
    <div class="space-y-2 px-3 pb-1">
      <div class="bg-base-200/30 rounded-lg p-2">
        <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Tokens Over Time</div>
        <div class="h-44 w-full">
          {#if tokTimeData.length > 0}
            <BarChart data={tokTimeData} x="bucket" series={tokTimeSeries} seriesLayout="stack" props={{ legend: { placement: 'bottom' } }} />
          {/if}
        </div>
      </div>
      <div class="grid grid-cols-3 gap-3">
        <!-- Token Type (input vs output) -->
        <div class="bg-base-200/30 rounded-lg p-2">
          <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Token Type</div>
          {#if tokenTypeSeries.length > 0}
            <div class="relative">
              <div class="h-32 w-full">
                <PieChart key="key" value="value" innerRadius={35} series={tokenTypeSeries} />
              </div>
              <div class="absolute inset-0 h-32 flex items-center justify-center pointer-events-none">
                <span class="text-sm font-bold text-base-content/70">{tokenTypeTotal}</span>
              </div>
            </div>
            <div class="flex flex-wrap gap-x-3 gap-y-0.5 mt-1 justify-center">
              {#each tokenTypeLegend as item}
                <div class="flex items-center gap-1 text-[10px]">
                  <span class="w-2 h-2 rounded-full shrink-0" style="background:{item.color}"></span>
                  <span class="text-base-content/50">{item.label}</span>
                  <span class="text-base-content/70 font-medium">{item.value}</span>
                </div>
              {/each}
            </div>
          {/if}
        </div>
        <!-- Tokens per model -->
        <div class="bg-base-200/30 rounded-lg p-2">
          <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Tokens Per Model</div>
          {#if modelTokenSeries.length > 0}
            <div class="relative">
              <div class="h-32 w-full">
                <PieChart key="key" value="value" innerRadius={35} series={modelTokenSeries} />
              </div>
              <div class="absolute inset-0 h-32 flex items-center justify-center pointer-events-none">
                <span class="text-sm font-bold text-base-content/70">{modelTokenTotal}</span>
              </div>
            </div>
            <div class="flex flex-col gap-0.5 mt-1">
              {#each modelTokenLegend as item}
                <div class="flex items-center gap-1 text-[10px]">
                  <span class="w-2 h-2 rounded-full shrink-0" style="background:{item.color}"></span>
                  <span class="text-base-content/50 truncate">{item.label}</span>
                  <span class="text-base-content/70 font-medium ml-auto shrink-0">{item.value}</span>
                </div>
              {/each}
            </div>
          {/if}
        </div>
        <!-- Cost per model -->
        <div class="bg-base-200/30 rounded-lg p-2">
          <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Cost Per Model</div>
          {#if modelCostSeries.length > 0}
            <div class="relative">
              <div class="h-32 w-full">
                <PieChart key="key" value="value" innerRadius={35} series={modelCostSeries} />
              </div>
              <div class="absolute inset-0 h-32 flex items-center justify-center pointer-events-none">
                <span class="text-sm font-bold text-base-content/70">{modelCostTotal}</span>
              </div>
            </div>
            <div class="flex flex-col gap-0.5 mt-1">
              {#each modelCostLegend as item}
                <div class="flex items-center gap-1 text-[10px]">
                  <span class="w-2 h-2 rounded-full shrink-0" style="background:{item.color}"></span>
                  <span class="text-base-content/50 truncate">{item.label}</span>
                  <span class="text-base-content/70 font-medium ml-auto shrink-0">{item.value}</span>
                </div>
              {/each}
            </div>
          {:else}
            <div class="flex items-center justify-center h-32 text-base-content/30 text-xs">No cost data</div>
          {/if}
        </div>
      </div>
    </div>

    <!-- Trace viewer -->
    {#if !loaded}
      <div class="flex items-center justify-center h-32"><span class="loading loading-spinner loading-md"></span></div>
    {:else if traces.length === 0}
      <div class="flex items-center justify-center h-32 text-base-content/40 text-sm">No traces recorded yet.</div>
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
            <tr class="hover:bg-base-200/40 cursor-pointer transition-colors border-b border-base-200" onclick={() => toggleTrace(trace.trace_id)}>
              <td class="pl-3 pr-1 py-2 w-5">
                <svg class="size-3 transition-transform {isExpanded ? 'rotate-90' : ''}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="9 18 15 12 9 6"/></svg>
              </td>
              <td class="py-2 pr-2 whitespace-nowrap font-medium {provColor(trace.provider)}">{trace.provider ?? 'unknown'}</td>
              <td class="py-2 pr-2 font-mono text-base-content/70">{trace.model ?? '?'}</td>
              <td class="py-2 pr-2 whitespace-nowrap text-base-content/50 text-right">{fmtTokens(trace.total_input_tokens)} in</td>
              <td class="py-2 pr-2 whitespace-nowrap text-base-content/50 text-right">{fmtTokens(trace.total_output_tokens)} out</td>
              <td class="py-2 pr-2 whitespace-nowrap text-base-content/40 text-right">{trace.total_tool_calls > 0 ? `${trace.total_tool_calls} tools` : ''}</td>
              <td class="py-2 pr-2 whitespace-nowrap text-base-content/40 font-mono text-right">{trace.total_duration_ms ? fmtDuration(trace.total_duration_ms) : ''}</td>
              <td class="py-2 pr-3 whitespace-nowrap text-base-content/40 font-mono text-right">${trace.total_cost.toFixed(3)}</td>
            </tr>
            {#if isExpanded}
              {#each calls as call}
                {@const toolCalls = callToolCalls(trace.trace_id, call.id)}
                {#if call.request_body_preview}
                  <tr class="bg-base-200/20 hover:bg-base-300/30 cursor-pointer transition-colors" onclick={() => selectInput(call)}>
                    <td></td>
                    <td class="py-1 pr-2"><span class="badge badge-xs w-10 text-center bg-info/15 text-info border-0">input</span></td>
                    <td class="py-1 text-base-content/50 whitespace-nowrap" colspan="6">{call.messages_count || '?'} messages, {call.tools_count || 0} tools</td>
                  </tr>
                {/if}
                {#if call.thinking_content}
                  <tr class="bg-base-200/20 hover:bg-base-300/30 cursor-pointer transition-colors" onclick={() => selectThinking(call)}>
                    <td></td>
                    <td class="py-1 pr-2"><span class="badge badge-xs w-10 text-center bg-span-thinking/15 text-span-thinking border-0">think</span></td>
                    <td class="py-1 text-base-content/50 truncate max-w-0" colspan="6">{truncate(call.thinking_content, 100)}</td>
                  </tr>
                {/if}
                {#each toolCalls as tc}
                  <tr class="bg-base-200/20 hover:bg-base-300/30 cursor-pointer transition-colors" onclick={() => selectTool(tc, trace.trace_id)}>
                    <td></td>
                    <td class="py-1 pr-2"><span class="badge badge-xs w-10 text-center bg-span-tool/15 text-span-tool border-0">tool</span></td>
                    <td class="py-1 font-mono text-base-content/70 whitespace-nowrap pr-2">{tc.tool_name}</td>
                    <td class="py-1 text-base-content/30 truncate max-w-0" colspan="5">{truncate(tc.arguments, 60)}</td>
                  </tr>
                {/each}
                {#if call.text_content}
                  <tr class="bg-base-200/20 hover:bg-base-300/30 cursor-pointer transition-colors" onclick={() => selectText(call)}>
                    <td></td>
                    <td class="py-1 pr-2"><span class="badge badge-xs w-10 text-center bg-span-answer/15 text-span-answer border-0">text</span></td>
                    <td class="py-1 text-base-content/50 truncate max-w-0" colspan="6">{truncate(call.text_content, 100)}</td>
                  </tr>
                {/if}
                {#if !call.request_body_preview && !call.thinking_content && !call.text_content && callToolCalls(trace.trace_id, call.id).length === 0}
                  <tr class="bg-base-200/20"><td></td><td class="py-1 pr-2"></td><td class="py-1 text-base-content/30 italic" colspan="6">no content captured</td></tr>
                {/if}
              {/each}
            {/if}
          {/each}
        </tbody>
      </table>
    {/if}
    </div>
  </div>

  {#if detail}
    <DetailPanel selection={detail} onClose={() => { detail = null; }} />
  {/if}
</div>
