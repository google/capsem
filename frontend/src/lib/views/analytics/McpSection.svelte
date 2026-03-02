<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import {
    Chart,
    BarController,
    BarElement,
    CategoryScale,
    LinearScale,
    Tooltip,
    Legend,
  } from 'chart.js';
  import { queryOne, queryAll } from '../../db';
  import { MCP_STATS_SQL, MCP_BY_SERVER_SQL, MCP_CALLS_SQL } from '../../sql';
  import type { McpCall, McpServerCallCount } from '../../types';

  Chart.register(BarController, BarElement, CategoryScale, LinearScale, Tooltip, Legend);

  // Provider/server color palette
  const SERVER_COLORS = [
    'rgba(59, 130, 246, 0.85)',
    'rgba(249, 115, 22, 0.85)',
    'rgba(34, 197, 94, 0.85)',
    'rgba(239, 68, 68, 0.85)',
    'rgba(139, 92, 246, 0.85)',
    'rgba(14, 165, 233, 0.85)',
    'rgba(99, 102, 241, 0.85)',
    'rgba(168, 85, 247, 0.85)',
  ];

  interface McpStatsRow {
    total: number;
    allowed: number;
    warned: number;
    denied: number;
    errored: number;
  }

  let calls = $state<McpCall[]>([]);
  let mcpStats = $state<McpStatsRow | null>(null);
  let byServer = $state<McpServerCallCount[]>([]);
  let pollTimer: ReturnType<typeof setInterval> | null = null;

  // Canvas refs
  let timeCanvas = $state<HTMLCanvasElement | undefined>();
  let serversCanvas = $state<HTMLCanvasElement | undefined>();
  let toolsCanvas = $state<HTMLCanvasElement | undefined>();

  // Chart instances
  let timeChart: Chart | null = null;
  let serversChart: Chart | null = null;
  let toolsChart: Chart | null = null;

  async function refresh() {
    try {
      const [c, s, srv] = await Promise.all([
        queryAll<McpCall>(MCP_CALLS_SQL, [100]),
        queryOne<McpStatsRow>(MCP_STATS_SQL),
        queryAll<McpServerCallCount>(MCP_BY_SERVER_SQL),
      ]);
      calls = c;
      mcpStats = s;
      byServer = srv;
    } catch {
      // backend not ready yet
    }
  }

  onMount(() => {
    refresh();
    pollTimer = setInterval(refresh, 2000);
  });

  onDestroy(() => {
    if (pollTimer) clearInterval(pollTimer);
    timeChart?.destroy();
    serversChart?.destroy();
    toolsChart?.destroy();
  });

  const gridColor = 'rgba(128,128,128,0.1)';
  const tickColor = 'rgba(128,128,128,0.6)';
  const tickFont = { size: 10 as const };
  const monoFont = { size: 10 as const, family: 'monospace' };

  // Server color map
  const serverColorMap = $derived.by(() => {
    const map = new Map<string, string>();
    byServer.forEach((srv, i) => {
      map.set(srv.server_name, SERVER_COLORS[i % SERVER_COLORS.length]);
    });
    return map;
  });

  // Calls over time -- bucket by relative time
  const timeBuckets = $derived.by(() => {
    if (calls.length === 0 || !mcpStats) return { labels: [] as string[], datasets: [] as { label: string; data: number[]; backgroundColor: string }[] };
    const sorted = [...calls].sort((a, b) => a.timestamp - b.timestamp);
    const servers = new Set<string>();
    for (const c of sorted) servers.add(c.server_name);
    const serverArr = Array.from(servers);

    const bucketSize = 5;
    const labels: string[] = [];
    const serverData = new Map<string, number[]>();
    for (const s of serverArr) serverData.set(s, []);

    for (let i = 0; i < sorted.length; i += bucketSize) {
      const slice = sorted.slice(i, i + bucketSize);
      labels.push(`#${Math.floor(i / bucketSize) + 1}`);
      for (const s of serverArr) {
        serverData.get(s)!.push(slice.filter(c => c.server_name === s).length);
      }
    }

    return {
      labels,
      datasets: serverArr.map(s => ({
        label: s,
        data: serverData.get(s)!,
        backgroundColor: serverColorMap.get(s) ?? 'rgba(139, 92, 246, 0.85)',
      })),
    };
  });

  $effect(() => {
    if (!timeCanvas || timeBuckets.labels.length === 0) return;
    if (timeChart) {
      timeChart.data.labels = timeBuckets.labels;
      timeChart.data.datasets = timeBuckets.datasets.map(ds => ({ ...ds, borderRadius: 2, borderSkipped: false }));
      timeChart.update('none');
      return;
    }
    timeChart = new Chart(timeCanvas, {
      type: 'bar',
      data: {
        labels: timeBuckets.labels,
        datasets: timeBuckets.datasets.map(ds => ({ ...ds, borderRadius: 2, borderSkipped: false })),
      },
      options: {
        responsive: true, maintainAspectRatio: false, animation: { duration: 400 },
        scales: {
          x: { stacked: true, grid: { display: false }, ticks: { color: tickColor, font: monoFont } },
          y: { stacked: true, beginAtZero: true, grid: { color: gridColor }, ticks: { color: tickColor, font: tickFont, stepSize: 1 } },
        },
        plugins: {
          legend: { display: true, position: 'bottom', labels: { color: tickColor, font: tickFont, boxWidth: 12, boxHeight: 10, padding: 12 } },
          tooltip: { callbacks: { label: (item) => `${item.dataset.label}: ${item.raw} calls` } },
        },
      },
    });
  });

  // Top servers chart
  $effect(() => {
    if (!serversCanvas || byServer.length === 0) return;
    const servers = byServer;
    if (serversChart) {
      serversChart.data.labels = servers.map(s => s.server_name);
      serversChart.data.datasets[0].data = servers.map(s => s.count);
      serversChart.data.datasets[0].backgroundColor = servers.map((s) => serverColorMap.get(s.server_name) ?? 'rgba(139, 92, 246, 0.85)');
      serversChart.update('none');
      return;
    }
    serversChart = new Chart(serversCanvas, {
      type: 'bar',
      data: {
        labels: servers.map(s => s.server_name),
        datasets: [{
          label: 'Calls',
          data: servers.map(s => s.count),
          backgroundColor: servers.map((s) => serverColorMap.get(s.server_name) ?? 'rgba(139, 92, 246, 0.85)'),
          borderRadius: 3, borderSkipped: false,
        }],
      },
      options: {
        responsive: true, maintainAspectRatio: false, animation: { duration: 400 },
        scales: {
          x: { grid: { display: false }, ticks: { color: tickColor, font: monoFont } },
          y: { beginAtZero: true, grid: { color: gridColor }, ticks: { color: tickColor, font: tickFont, stepSize: 1 } },
        },
        plugins: { legend: { display: false }, tooltip: { callbacks: { label: (item) => `${item.raw} calls` } } },
      },
    });
  });

  // Top tools chart (colored by server)
  const toolData = $derived.by(() => {
    const counts = new Map<string, { count: number; server: string }>();
    for (const c of calls) {
      if (!c.tool_name) continue;
      const existing = counts.get(c.tool_name);
      if (existing) {
        existing.count++;
      } else {
        counts.set(c.tool_name, { count: 1, server: c.server_name });
      }
    }
    return Array.from(counts.entries())
      .map(([name, { count, server }]) => ({ name, count, server }))
      .sort((a, b) => b.count - a.count)
      .slice(0, 8);
  });

  $effect(() => {
    if (!toolsCanvas || toolData.length === 0) return;
    if (toolsChart) {
      toolsChart.data.labels = toolData.map(t => t.name);
      toolsChart.data.datasets[0].data = toolData.map(t => t.count);
      toolsChart.data.datasets[0].backgroundColor = toolData.map(t => serverColorMap.get(t.server) ?? 'rgba(139, 92, 246, 0.85)');
      toolsChart.update('none');
      return;
    }
    toolsChart = new Chart(toolsCanvas, {
      type: 'bar',
      data: {
        labels: toolData.map(t => t.name),
        datasets: [{
          label: 'Calls',
          data: toolData.map(t => t.count),
          backgroundColor: toolData.map(t => serverColorMap.get(t.server) ?? 'rgba(139, 92, 246, 0.85)'),
          borderRadius: 3, borderSkipped: false,
        }],
      },
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
</script>

<div class="space-y-6">
  <!-- Top stat cards -->
  <div class="grid grid-cols-4 gap-3">
    <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
      <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Total Calls</div>
      <div class="mt-1 text-xl font-semibold tabular-nums">{mcpStats?.total ?? 0}</div>
    </div>
    <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
      <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Allowed</div>
      <div class="mt-1 text-xl font-semibold tabular-nums text-info">{mcpStats?.allowed ?? 0}</div>
    </div>
    <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
      <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Warned</div>
      <div class="mt-1 text-xl font-semibold tabular-nums text-warning">{mcpStats?.warned ?? 0}</div>
    </div>
    <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
      <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Denied</div>
      <div class="mt-1 text-xl font-semibold tabular-nums text-secondary">{mcpStats?.denied ?? 0}</div>
    </div>
  </div>

  <!-- Calls over time -->
  {#if calls.length > 0}
    <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
      <h4 class="text-xs font-semibold text-base-content/60 mb-2">Calls over time</h4>
      <div class="h-48"><canvas bind:this={timeCanvas}></canvas></div>
    </div>
  {/if}

  <!-- Bottom: top servers + top tools -->
  {#if byServer.length > 0}
    <div class="grid grid-cols-2 gap-4">
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <h4 class="text-xs font-semibold text-base-content/60 mb-2">Top servers</h4>
        <div class="h-48"><canvas bind:this={serversCanvas}></canvas></div>
      </div>
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <h4 class="text-xs font-semibold text-base-content/60 mb-2">Top tools</h4>
        {#if toolData.length > 0}
          <div class="h-48"><canvas bind:this={toolsCanvas}></canvas></div>
        {:else}
          <div class="flex items-center justify-center h-48 text-[10px] text-base-content/40">No tool calls recorded</div>
        {/if}
      </div>
    </div>
  {:else if calls.length === 0}
    <div class="flex items-center justify-center rounded-lg border border-base-300 bg-base-200/50 py-8">
      <span class="text-sm text-base-content/40">No MCP calls recorded yet</span>
    </div>
  {/if}
</div>
