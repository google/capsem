<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import {
    Chart,
    BarController,
    BarElement,
    CategoryScale,
    LinearScale,
    DoughnutController,
    ArcElement,
    Tooltip,
    Legend,
  } from 'chart.js';
  import { getSessionInfo, getTraces, getTraceDetail } from '../api';
  import { networkStore } from '../stores/network.svelte';
  import TraceDetailView, { type FlatItem } from '../components/TraceDetail.svelte';
  import type { SessionInfo, TraceSummary, TraceDetail as TraceDetailT, TraceModelCall, NetEvent, ToolCallEntry, ToolResponseEntry } from '../types';

  Chart.register(BarController, BarElement, CategoryScale, LinearScale, DoughnutController, ArcElement, Tooltip, Legend);

  let info = $state<SessionInfo | null>(null);
  let traces = $state<TraceSummary[]>([]);
  let error = $state<string | null>(null);
  let traceLimit = $state(10);

  // Inline trace expansion
  let expandedTraceId = $state<string | null>(null);
  let expandedDetail = $state<TraceDetailT | null>(null);
  let detailLoading = $state(false);

  // Canvas refs
  let tokenCanvas = $state<HTMLCanvasElement | undefined>();
  let toolCanvas = $state<HTMLCanvasElement | undefined>();
  let modelCanvas = $state<HTMLCanvasElement | undefined>();
  let timeCanvas = $state<HTMLCanvasElement | undefined>();
  let domainCanvas = $state<HTMLCanvasElement | undefined>();
  let methodCanvas = $state<HTMLCanvasElement | undefined>();

  // Chart instances
  let tokenChart: Chart | null = null;
  let toolChart: Chart | null = null;
  let modelChart: Chart | null = null;
  let timeChart: Chart | null = null;
  let domainChart: Chart | null = null;
  let methodChart: Chart | null = null;

  // Sliding panel state
  type PanelData =
    | { kind: 'item'; item: FlatItem }
    | { kind: 'http'; event: NetEvent };

  let panel = $state<PanelData | null>(null);
  let selectedItem = $state<FlatItem | null>(null);

  onMount(async () => {
    try {
      [info, traces] = await Promise.all([
        getSessionInfo(),
        getTraces(50),
      ]);
    } catch (e) {
      error = String(e);
    }
  });

  // -- Inline trace expansion --
  async function toggleTrace(traceId: string) {
    if (expandedTraceId === traceId) {
      expandedTraceId = null;
      expandedDetail = null;
      // Close panel if it was showing an item from this trace
      if (panel?.kind === 'item') {
        panel = null;
        selectedItem = null;
      }
      return;
    }
    expandedTraceId = traceId;
    expandedDetail = null;
    detailLoading = true;
    try {
      expandedDetail = await getTraceDetail(traceId);
    } catch {
      expandedDetail = null;
    }
    detailLoading = false;
  }

  // -- Panel actions --
  function openItem(item: FlatItem) {
    selectedItem = item;
    panel = { kind: 'item', item };
  }

  function openHttp(event: NetEvent) {
    panel = { kind: 'http', event };
  }

  function closePanel() {
    panel = null;
    selectedItem = null;
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && panel) closePanel();
  }

  // -- Shared chart config --
  const gridColor = 'rgba(128,128,128,0.1)';
  const tickColor = 'rgba(128,128,128,0.6)';
  const tickFont = { size: 10 as const };
  const monoFont = { size: 10 as const, family: 'monospace' };
  const BLUE = 'rgba(59, 130, 246, 0.85)';
  const BLUE_LIGHT = 'rgba(147, 197, 253, 0.7)';
  const PURPLE = 'rgba(139, 92, 246, 0.85)';

  const METHOD_COLORS = [
    BLUE, PURPLE, 'rgba(14, 165, 233, 0.85)', 'rgba(99, 102, 241, 0.85)',
    'rgba(168, 85, 247, 0.85)', 'rgba(6, 182, 212, 0.85)',
  ];

  function tokenTickCallback(v: string | number) {
    const n = Number(v);
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(n >= 10_000 ? 0 : 1)}K`;
    return String(n);
  }

  // -- Tokens per round chart --
  $effect(() => {
    if (!tokenCanvas || traces.length === 0) return;
    if (tokenChart) return;
    const recent = [...traces].reverse().slice(-10);
    tokenChart = new Chart(tokenCanvas, {
      type: 'bar',
      data: {
        labels: recent.map((_, i) => `#${i + 1}`),
        datasets: [
          { label: 'Input tokens', data: recent.map(t => t.total_input_tokens), backgroundColor: BLUE },
          { label: 'Output tokens', data: recent.map(t => t.total_output_tokens), backgroundColor: BLUE_LIGHT },
        ],
      },
      options: {
        responsive: true, maintainAspectRatio: false, animation: { duration: 400 },
        scales: {
          x: { stacked: true, grid: { display: false }, ticks: { color: tickColor, font: monoFont } },
          y: { stacked: true, beginAtZero: true, grid: { color: gridColor }, ticks: { color: tickColor, font: tickFont, callback: tokenTickCallback } },
        },
        plugins: {
          legend: { display: true, position: 'bottom', labels: { color: tickColor, font: tickFont, boxWidth: 12, boxHeight: 10, padding: 12 } },
          tooltip: {
            callbacks: {
              title: (items) => { const t = recent[items[0].dataIndex]; return `#${items[0].dataIndex + 1} - ${t.model ?? t.provider}`; },
              afterTitle: (items) => { const t = recent[items[0].dataIndex]; return `${formatCost(t.total_estimated_cost_usd)} | ${t.total_duration_ms}ms`; },
            },
          },
        },
      },
    });
  });

  // -- Top tools chart --
  const toolUsage = $derived(networkStore.stats?.tool_usage ?? []);

  $effect(() => {
    if (!toolCanvas || toolUsage.length === 0) return;
    const tools = toolUsage.slice(0, 8);
    const labels = tools.map(t => t.tool_name);
    const data = tools.map(t => t.count);
    if (toolChart) {
      toolChart.data.labels = labels;
      toolChart.data.datasets[0].data = data;
      toolChart.update('none');
      return;
    }
    toolChart = new Chart(toolCanvas, {
      type: 'bar',
      data: { labels, datasets: [{ label: 'Calls', data, backgroundColor: BLUE, borderRadius: 3, borderSkipped: false }] },
      options: {
        responsive: true, maintainAspectRatio: false, animation: { duration: 400 },
        scales: {
          x: { grid: { display: false }, ticks: { color: tickColor, font: monoFont, maxRotation: 45, minRotation: 0 } },
          y: { beginAtZero: true, grid: { color: gridColor }, ticks: { color: tickColor, font: tickFont, stepSize: 1 } },
        },
        plugins: { legend: { display: false }, tooltip: { callbacks: { label: (item) => `${item.raw} calls` } } },
      },
    });
  });

  // -- Model usage doughnut --
  const MODEL_COLORS = [BLUE, PURPLE, 'rgba(14, 165, 233, 0.85)', 'rgba(99, 102, 241, 0.85)', 'rgba(168, 85, 247, 0.85)', 'rgba(6, 182, 212, 0.85)'];

  const modelUsage = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const t of traces) {
      const m = t.model ?? t.provider;
      counts.set(m, (counts.get(m) ?? 0) + 1);
    }
    return Array.from(counts.entries()).sort((a, b) => b[1] - a[1]).map(([model, count]) => ({ model, count }));
  });

  $effect(() => {
    if (!modelCanvas || modelUsage.length === 0) return;
    if (modelChart) return;
    modelChart = new Chart(modelCanvas, {
      type: 'doughnut',
      data: {
        labels: modelUsage.map(m => m.model),
        datasets: [{ data: modelUsage.map(m => m.count), backgroundColor: modelUsage.map((_, i) => MODEL_COLORS[i % MODEL_COLORS.length]), borderWidth: 2, borderColor: 'rgba(0,0,0,0.1)' }],
      },
      options: {
        responsive: true, maintainAspectRatio: false, cutout: '55%', animation: { duration: 400 },
        plugins: {
          legend: { display: true, position: 'right', labels: { color: tickColor, font: tickFont, boxWidth: 10, boxHeight: 10, padding: 8 } },
          tooltip: { callbacks: { label: (item) => { const total = modelUsage.reduce((s, m) => s + m.count, 0); const pct = total > 0 ? ((Number(item.raw) / total) * 100).toFixed(0) : '0'; return `${item.label}: ${item.raw} (${pct}%)`; } } },
        },
      },
    });
  });

  // -- Network: Requests over time chart --
  const timeBuckets = $derived(networkStore.stats?.time_buckets ?? []);

  $effect(() => {
    if (!timeCanvas || timeBuckets.length === 0) return;
    const labels = timeBuckets.map((_, i) => i === timeBuckets.length - 1 ? 'now' : `${timeBuckets.length - 1 - i}m`);
    const allowedData = timeBuckets.map(b => b.allowed);
    const deniedData = timeBuckets.map(b => b.denied);
    if (timeChart) {
      timeChart.data.labels = labels;
      timeChart.data.datasets[0].data = allowedData;
      timeChart.data.datasets[1].data = deniedData;
      timeChart.update('none');
      return;
    }
    timeChart = new Chart(timeCanvas, {
      type: 'bar',
      data: {
        labels,
        datasets: [
          { label: 'Allowed', data: allowedData, backgroundColor: BLUE, borderRadius: 2, borderSkipped: false },
          { label: 'Denied', data: deniedData, backgroundColor: PURPLE, borderRadius: 2, borderSkipped: false },
        ],
      },
      options: {
        responsive: true, maintainAspectRatio: false, animation: { duration: 400 },
        scales: {
          x: { stacked: true, grid: { display: false }, ticks: { color: tickColor, font: monoFont } },
          y: { stacked: true, beginAtZero: true, grid: { color: gridColor }, ticks: { color: tickColor, font: tickFont, stepSize: 1 } },
        },
        plugins: {
          legend: { display: true, position: 'bottom', labels: { color: tickColor, font: tickFont, boxWidth: 12, boxHeight: 10, padding: 12 } },
          tooltip: { callbacks: { label: (item) => `${item.dataset.label}: ${item.raw}` } },
        },
      },
    });
  });

  // -- Network: Domains bar chart --
  const domainCounts = $derived(networkStore.stats?.top_domains ?? []);

  $effect(() => {
    if (!domainCanvas || domainCounts.length === 0) return;
    const domains = domainCounts.slice(0, 8);
    const labels = domains.map(d => d.domain.length > 20 ? d.domain.slice(0, 18) + '..' : d.domain);
    const allowedData = domains.map(d => d.allowed);
    const deniedData = domains.map(d => d.denied);
    if (domainChart) {
      domainChart.data.labels = labels;
      domainChart.data.datasets[0].data = allowedData;
      domainChart.data.datasets[1].data = deniedData;
      domainChart.update('none');
      return;
    }
    domainChart = new Chart(domainCanvas, {
      type: 'bar',
      data: {
        labels,
        datasets: [
          { label: 'Allowed', data: allowedData, backgroundColor: BLUE, borderRadius: 2, borderSkipped: false },
          { label: 'Denied', data: deniedData, backgroundColor: PURPLE, borderRadius: 2, borderSkipped: false },
        ],
      },
      options: {
        responsive: true, maintainAspectRatio: false, animation: { duration: 400 },
        scales: {
          x: { stacked: true, grid: { display: false }, ticks: { color: tickColor, font: monoFont, maxRotation: 45, minRotation: 0 } },
          y: { stacked: true, beginAtZero: true, grid: { color: gridColor }, ticks: { color: tickColor, font: tickFont, stepSize: 1 } },
        },
        plugins: {
          legend: { display: false },
          tooltip: {
            callbacks: {
              title: (items) => domainCounts[items[0].dataIndex]?.domain ?? '',
              label: (item) => `${item.dataset.label}: ${item.raw}`,
            },
          },
        },
      },
    });
  });

  // -- Network: HTTP methods doughnut --
  const methodCounts = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const e of networkStore.events) {
      const m = e.method ?? 'CONNECT';
      counts.set(m, (counts.get(m) ?? 0) + 1);
    }
    return Array.from(counts.entries()).sort((a, b) => b[1] - a[1]);
  });

  $effect(() => {
    if (!methodCanvas || methodCounts.length === 0) return;
    const labels = methodCounts.map(([m]) => m);
    const data = methodCounts.map(([, c]) => c);
    if (methodChart) {
      methodChart.data.labels = labels;
      methodChart.data.datasets[0].data = data;
      methodChart.data.datasets[0].backgroundColor = labels.map((_, i) => METHOD_COLORS[i % METHOD_COLORS.length]);
      methodChart.update('none');
      return;
    }
    methodChart = new Chart(methodCanvas, {
      type: 'doughnut',
      data: {
        labels,
        datasets: [{
          data,
          backgroundColor: labels.map((_, i) => METHOD_COLORS[i % METHOD_COLORS.length]),
          borderWidth: 2,
          borderColor: 'rgba(0,0,0,0.1)',
        }],
      },
      options: {
        responsive: true, maintainAspectRatio: false, cutout: '50%', animation: { duration: 400 },
        plugins: {
          legend: { display: true, position: 'right', labels: { color: tickColor, font: tickFont, boxWidth: 10, boxHeight: 10, padding: 8 } },
          tooltip: { callbacks: { label: (item) => { const total = methodCounts.reduce((s, [, c]) => s + c, 0); const pct = total > 0 ? ((Number(item.raw) / total) * 100).toFixed(0) : '0'; return `${item.label}: ${item.raw} (${pct}%)`; } } },
        },
      },
    });
  });

  onDestroy(() => {
    tokenChart?.destroy();
    toolChart?.destroy();
    modelChart?.destroy();
    timeChart?.destroy();
    domainChart?.destroy();
    methodChart?.destroy();
  });

  // -- Helpers --
  function formatTime(ts: number | string): string {
    const d = typeof ts === 'string' ? new Date(ts) : new Date(ts * 1000);
    return d.toLocaleTimeString();
  }

  function formatCost(usd: number): string {
    if (usd === 0) return '$0.00';
    if (usd < 0.01) return `$${usd.toFixed(4)}`;
    return `$${usd.toFixed(2)}`;
  }

  function formatTokens(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return `${n}`;
  }

  function formatBytes(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)} MB`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)} KB`;
    return `${n} B`;
  }

  function formatJson(s: string | null): string {
    if (!s) return '';
    try { return JSON.stringify(JSON.parse(s), null, 2); } catch { return s; }
  }

  function decisionBadge(decision: string): string {
    switch (decision) {
      case 'allowed': return 'badge-info';
      case 'denied': return 'badge-secondary';
      default: return 'badge-warning';
    }
  }

  function methodColor(method: string | null): string {
    switch (method) {
      case 'GET': return 'text-info';
      case 'POST': return 'text-success';
      case 'PUT': case 'PATCH': return 'text-warning';
      case 'DELETE': return 'text-secondary';
      default: return 'text-base-content/50';
    }
  }

  /** Tool calls are on generation i; responses are on generation i+1. */
  function findToolResponse(callId: string, genIndex: number, detail: TraceDetailT): ToolResponseEntry | null {
    const nextGen = detail.calls[genIndex + 1];
    if (!nextGen) return null;
    return nextGen.tool_responses.find(r => r.call_id === callId) ?? null;
  }

  // -- Pagination --
  let tableLimit = $state(10);
  const tableEvents = $derived(networkStore.events.slice(0, tableLimit));
  const hasMore = $derived(networkStore.events.length > tableLimit);
  const visibleTraces = $derived(traces.slice(0, traceLimit));
  const hasMoreTraces = $derived(traces.length > traceLimit);
</script>

<svelte:window on:keydown={onKeydown} />

<div class="relative flex h-full overflow-hidden">
  <!-- Main scrollable content -->
  <div
    class="flex-1 overflow-auto transition-[margin] duration-200"
    style:margin-right={panel ? '480px' : '0px'}
  >
    <div class="p-4 space-y-6">
      {#if info}
        <!-- Stat cards -->
        <div class="grid grid-cols-4 gap-3">
          <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
            <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Generations</div>
            <div class="mt-1 text-xl font-semibold tabular-nums">{info.model_call_count}</div>
            <div class="mt-1 text-[10px] text-base-content/50">{info.total_tool_calls} tool calls</div>
          </div>
          <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
            <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Tokens</div>
            <div class="mt-1 text-xl font-semibold tabular-nums">{formatTokens(info.total_input_tokens + info.total_output_tokens)}</div>
            <div class="mt-1 flex gap-2 text-[10px] text-base-content/50">
              <span>{formatTokens(info.total_input_tokens)} in</span>
              <span>{formatTokens(info.total_output_tokens)} out</span>
            </div>
          </div>
          <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
            <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Cost</div>
            <div class="mt-1 text-xl font-semibold tabular-nums text-info">{formatCost(info.total_estimated_cost_usd)}</div>
          </div>
          <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
            <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">HTTPS</div>
            <div class="mt-1 text-xl font-semibold tabular-nums">{info.total_requests}</div>
            <div class="mt-1 flex gap-2 text-[10px]">
              <span class="text-info">{info.allowed_requests} allowed</span>
              <span class="text-secondary">{info.denied_requests} denied</span>
            </div>
          </div>
        </div>

        {#if traces.length > 0}
          <!-- Tokens per round -->
          <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
            <h4 class="text-xs font-semibold text-base-content/60 mb-2">Tokens per round</h4>
            <div class="h-52"><canvas bind:this={tokenCanvas}></canvas></div>
          </div>

          <!-- Top tools + Model usage -->
          <div class="grid grid-cols-2 gap-4">
            <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
              <h4 class="text-xs font-semibold text-base-content/60 mb-2">Top tools</h4>
              {#if toolUsage.length > 0}
                <div class="h-48"><canvas bind:this={toolCanvas}></canvas></div>
              {:else}
                <div class="flex items-center justify-center h-48 text-[10px] text-base-content/40">No tool calls recorded</div>
              {/if}
            </div>
            <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
              <h4 class="text-xs font-semibold text-base-content/60 mb-2">Models</h4>
              {#if modelUsage.length > 0}
                <div class="h-48"><canvas bind:this={modelCanvas}></canvas></div>
              {:else}
                <div class="flex items-center justify-center h-48 text-[10px] text-base-content/40">No model data</div>
              {/if}
            </div>
          </div>

          <!-- Spans table with inline trace expansion -->
          <div>
            <h3 class="text-sm font-semibold mb-3">Spans</h3>
            <div class="rounded-lg border border-base-300 overflow-hidden">
              <table class="table table-zebra table-xs w-full">
                <thead class="bg-base-200">
                  <tr>
                    <th class="w-6"></th>
                    <th>Time</th>
                    <th>Model</th>
                    <th class="text-right">Generations</th>
                    <th class="text-right">Tools</th>
                    <th class="text-right">In</th>
                    <th class="text-right">Out</th>
                    <th class="text-right">Cost</th>
                    <th class="text-right">Duration</th>
                  </tr>
                </thead>
                <tbody>
                  {#each visibleTraces as trace}
                    <tr
                      class="cursor-pointer hover:bg-base-200/80"
                      onclick={() => toggleTrace(trace.trace_id)}
                    >
                      <td class="text-base-content/40">{expandedTraceId === trace.trace_id ? '\u25BC' : '\u25B6'}</td>
                      <td class="font-mono">{formatTime(trace.started_at)}</td>
                      <td class="font-mono text-xs max-w-32 truncate" title={trace.model ?? ''}>{trace.model ?? '-'}</td>
                      <td class="text-right tabular-nums">{trace.call_count}</td>
                      <td class="text-right tabular-nums">{trace.total_tool_calls}</td>
                      <td class="text-right tabular-nums">{formatTokens(trace.total_input_tokens)}</td>
                      <td class="text-right tabular-nums">{formatTokens(trace.total_output_tokens)}</td>
                      <td class="text-right tabular-nums text-info">{formatCost(trace.total_estimated_cost_usd)}</td>
                      <td class="text-right tabular-nums">{trace.total_duration_ms}ms</td>
                    </tr>
                    {#if expandedTraceId === trace.trace_id}
                      <tr>
                        <td colspan="9" class="p-0 bg-base-100">
                          <div class="p-4">
                            {#if detailLoading}
                              <div class="flex justify-center py-4">
                                <span class="loading loading-spinner loading-sm"></span>
                              </div>
                            {:else if expandedDetail}
                              <TraceDetailView
                                detail={expandedDetail}
                                onSelectItem={openItem}
                                {selectedItem}
                                {findToolResponse}
                              />
                            {/if}
                          </div>
                        </td>
                      </tr>
                    {/if}
                  {/each}
                </tbody>
              </table>
              {#if hasMoreTraces}
                <div class="flex justify-center border-t border-base-300 bg-base-200/50 py-2">
                  <button class="btn btn-xs btn-ghost text-base-content/60" onclick={() => traceLimit += 10}>
                    Show more ({traces.length - traceLimit} remaining)
                  </button>
                </div>
              {/if}
            </div>
          </div>
        {:else if info.model_call_count > 0}
          <div class="flex items-center justify-center rounded-lg border border-base-300 bg-base-200/50 py-8">
            <span class="text-sm text-base-content/40">Model calls exist but no traces yet (data predates trace grouping)</span>
          </div>
        {:else}
          <div class="flex items-center justify-center rounded-lg border border-base-300 bg-base-200/50 py-8">
            <span class="text-sm text-base-content/40">No LLM calls recorded yet</span>
          </div>
        {/if}

        <!-- Network Analytics -->
        <div>
          <h3 class="text-sm font-semibold mb-3">Network</h3>

          {#if networkStore.totalCalls > 0}
            <!-- Requests over time -->
            <div class="rounded-lg border border-base-300 bg-base-200/50 p-3 mb-4">
              <h4 class="text-xs font-semibold text-base-content/60 mb-2">Requests over time</h4>
              <div class="h-40"><canvas bind:this={timeCanvas}></canvas></div>
            </div>

            <!-- Domains (2/3) + Methods pie (1/3) -->
            <div class="grid grid-cols-3 gap-4 mb-4">
              <div class="col-span-2 rounded-lg border border-base-300 bg-base-200/50 p-3">
                <h4 class="text-xs font-semibold text-base-content/60 mb-2">Domains</h4>
                <div class="h-48"><canvas bind:this={domainCanvas}></canvas></div>
              </div>
              <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
                <h4 class="text-xs font-semibold text-base-content/60 mb-2">HTTP Methods</h4>
                <div class="h-48"><canvas bind:this={methodCanvas}></canvas></div>
              </div>
            </div>

            <!-- Network events table -->
            <div class="rounded-lg border border-base-300 overflow-hidden">
              <table class="table table-zebra table-xs w-full">
                <thead class="bg-base-200">
                  <tr>
                    <th>Time</th>
                    <th>Process</th>
                    <th>Domain</th>
                    <th>Method</th>
                    <th>Path</th>
                    <th>Status</th>
                    <th>Decision</th>
                    <th>Duration</th>
                  </tr>
                </thead>
                <tbody>
                  {#each tableEvents as event}
                    <tr
                      class="cursor-pointer hover:bg-base-200/80 {panel?.kind === 'http' && panel.event === event ? 'bg-info/10' : ''}"
                      onclick={() => openHttp(event)}
                    >
                      <td class="font-mono">{formatTime(event.timestamp)}</td>
                      <td class="text-xs text-base-content/60">
                        {#if event.process_name}
                          <span class="font-mono">{event.process_name}</span>
                          {#if event.pid}<span class="text-base-content/30 ml-0.5">:{event.pid}</span>{/if}
                        {:else}
                          <span class="text-base-content/30">-</span>
                        {/if}
                      </td>
                      <td class="max-w-40 truncate">{event.domain}</td>
                      <td><span class="font-mono text-xs {methodColor(event.method)}">{event.method ?? 'CONNECT'}</span></td>
                      <td class="max-w-48 truncate font-mono">{event.path ?? '-'}</td>
                      <td>{event.status_code ?? '-'}</td>
                      <td><span class="badge badge-xs {decisionBadge(event.decision)}">{event.decision}</span></td>
                      <td class="tabular-nums">{event.duration_ms}ms</td>
                    </tr>
                  {/each}
                </tbody>
              </table>
              {#if hasMore}
                <div class="flex justify-center border-t border-base-300 bg-base-200/50 py-2">
                  <button class="btn btn-xs btn-ghost text-base-content/60" onclick={() => tableLimit += 10}>
                    Show more ({networkStore.events.length - tableLimit} remaining)
                  </button>
                </div>
              {/if}
            </div>
          {:else}
            <div class="flex items-center justify-center rounded-lg border border-base-300 bg-base-200/50 py-8">
              <span class="text-sm text-base-content/40">No network events recorded</span>
            </div>
          {/if}
        </div>

      {:else if error}
        <div class="flex items-center justify-center h-32 text-base-content/40 text-sm">{error}</div>
      {:else}
        <div class="flex items-center justify-center h-32">
          <span class="loading loading-spinner loading-md"></span>
        </div>
      {/if}
    </div>
  </div>

  <!-- Sliding detail panel -->
  <div
    class="absolute top-0 right-0 bottom-0 w-[480px] border-l border-base-300 bg-base-100
           flex flex-col shadow-xl z-10 transform transition-transform duration-200 ease-out
           {panel ? 'translate-x-0' : 'translate-x-full'}"
  >
    {#if panel}
      <!-- Panel header -->
      <div class="flex items-center gap-3 px-4 py-3 border-b border-base-300 bg-base-200/50 shrink-0">
        {#if panel.kind === 'item'}
          {@const item = panel.item}
          {#if item.type === 'thinking'}
            <h3 class="text-sm font-semibold italic">Thinking</h3>
          {:else if item.type === 'output'}
            <h3 class="text-sm font-semibold">Output</h3>
          {:else}
            <span class="badge badge-sm badge-info font-mono">{item.tool.tool_name}</span>
            {#if item.response?.is_error}
              <span class="badge badge-xs badge-secondary">error</span>
            {/if}
          {/if}
          <span class="text-xs text-base-content/50 tabular-nums">{item.call.input_tokens != null ? formatTokens(item.call.input_tokens) : '-'} in</span>
          <span class="text-xs text-base-content/50 tabular-nums">{item.call.output_tokens != null ? formatTokens(item.call.output_tokens) : '-'} out</span>
          <span class="text-xs text-info tabular-nums">{formatCost(item.call.estimated_cost_usd)}</span>
        {:else}
          <h3 class="text-sm font-semibold">HTTP Request</h3>
          <span class="badge badge-xs {decisionBadge(panel.event.decision)}">{panel.event.decision}</span>
        {/if}
        <button
          class="ml-auto text-base-content/30 hover:text-base-content/60 text-lg leading-none"
          onclick={closePanel}
          aria-label="Close panel"
        >&#10005;</button>
      </div>

      <!-- Panel body -->
      <div class="flex-1 overflow-auto p-4 space-y-4">
        {#if panel.kind === 'item'}
          {@const item = panel.item}

          {#if item.type === 'thinking'}
            <pre class="rounded bg-base-300/40 px-3 py-2 text-[11px] text-base-content/50 italic whitespace-pre-wrap break-words overflow-auto font-mono">{item.call.thinking_content}</pre>

          {:else if item.type === 'output'}
            <pre class="rounded bg-base-300/40 px-3 py-2 text-[11px] text-base-content/70 whitespace-pre-wrap break-words overflow-auto font-mono">{item.call.text_content}</pre>

          {:else if item.type === 'tool'}
            {#if item.tool.arguments}
              <div>
                <div class="text-[10px] text-base-content/30 uppercase tracking-wider mb-0.5">Arguments</div>
                <pre class="rounded bg-base-300/30 px-2.5 py-1.5 text-[11px] text-base-content/60 whitespace-pre-wrap break-words overflow-auto font-mono">{formatJson(item.tool.arguments)}</pre>
              </div>
            {/if}
            {#if item.response}
              <div>
                <div class="text-[10px] uppercase tracking-wider mb-0.5 {item.response.is_error ? 'text-secondary/60' : 'text-base-content/30'}">{item.response.is_error ? 'Error' : 'Result'}</div>
                <pre class="rounded px-2.5 py-1.5 text-[11px] whitespace-pre-wrap break-words overflow-auto font-mono {item.response.is_error ? 'bg-secondary/10 text-secondary' : 'bg-base-300/30 text-base-content/60'}">{item.response.content_preview ?? '(empty)'}</pre>
              </div>
            {/if}
          {/if}

        {:else if panel.kind === 'http'}
          {@const ev = panel.event}

          <div class="flex items-start gap-2">
            <span class="badge badge-sm font-mono {methodColor(ev.method)}">{ev.method ?? 'CONNECT'}</span>
            <span class="font-mono text-sm break-all text-base-content/80">{ev.path ?? '/'}{ev.query ? `?${ev.query}` : ''}</span>
          </div>

          <div class="grid grid-cols-2 gap-x-4 gap-y-2 text-xs">
            <div>
              <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider">Domain</div>
              <div class="font-mono">{ev.domain}:{ev.port}</div>
            </div>
            <div>
              <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider">Status</div>
              <div class="font-mono">{ev.status_code ?? '-'}</div>
            </div>
            <div>
              <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider">Process</div>
              <div class="font-mono">
                {#if ev.process_name}
                  {ev.process_name}
                  {#if ev.pid}<span class="text-base-content/40"> (PID {ev.pid})</span>{/if}
                {:else}
                  <span class="text-base-content/30">-</span>
                {/if}
              </div>
            </div>
            <div>
              <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider">Duration</div>
              <div class="font-mono">{ev.duration_ms}ms</div>
            </div>
            <div>
              <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider">Sent</div>
              <div class="font-mono">{formatBytes(ev.bytes_sent)}</div>
            </div>
            <div>
              <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider">Received</div>
              <div class="font-mono">{formatBytes(ev.bytes_received)}</div>
            </div>
            <div>
              <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider">Rule</div>
              <div class="font-mono text-base-content/60">{ev.matched_rule ?? '-'}</div>
            </div>
            <div>
              <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider">Connection</div>
              <div class="font-mono text-base-content/60">{ev.conn_type ?? '-'}</div>
            </div>
          </div>

          {#if ev.request_headers}
            <div>
              <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Request Headers</div>
              <pre class="rounded bg-base-300/40 px-3 py-2 text-[11px] text-base-content/60 whitespace-pre-wrap break-words max-h-32 overflow-auto font-mono">{ev.request_headers}</pre>
            </div>
          {/if}

          {#if ev.response_headers}
            <div>
              <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Response Headers</div>
              <pre class="rounded bg-base-300/40 px-3 py-2 text-[11px] text-base-content/60 whitespace-pre-wrap break-words max-h-32 overflow-auto font-mono">{ev.response_headers}</pre>
            </div>
          {/if}

          {#if ev.request_body_preview}
            <div>
              <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Request Body</div>
              <pre class="rounded bg-base-300/40 px-3 py-2 text-[11px] text-base-content/60 whitespace-pre-wrap break-words max-h-48 overflow-auto font-mono">{ev.request_body_preview}</pre>
            </div>
          {/if}

          {#if ev.response_body_preview}
            <div>
              <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Response Body</div>
              <pre class="rounded bg-base-300/40 px-3 py-2 text-[11px] text-base-content/60 whitespace-pre-wrap break-words max-h-48 overflow-auto font-mono">{ev.response_body_preview}</pre>
            </div>
          {/if}
        {/if}
      </div>
    {/if}
  </div>
</div>
