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
  import { queryAll, queryOne, getTraces, getSessionHistory } from '../../api';
  import { PROVIDER_USAGE_SQL, TOOL_USAGE_SQL, MODEL_STATS_SQL } from '../../sql';
  import type { ProviderTokenUsage, ToolUsageCount, TraceSummary, SessionRecord } from '../../types';

  Chart.register(BarController, BarElement, CategoryScale, LinearScale, DoughnutController, ArcElement, Tooltip, Legend);

  // Provider color palette (keyed by DB value)
  const PROVIDER_COLORS: Record<string, string> = {
    google: 'rgba(59, 130, 246, 0.85)',
    anthropic: 'rgba(249, 115, 22, 0.85)',
    openai: 'rgba(34, 197, 94, 0.85)',
    mistral: 'rgba(239, 68, 68, 0.85)',
  };
  const PROVIDER_DEFAULT = 'rgba(139, 92, 246, 0.85)';

  // DB stores "google", "anthropic", "openai" -- display as product names
  const PROVIDER_LABELS: Record<string, string> = {
    google: 'Gemini',
    anthropic: 'Claude',
    openai: 'OpenAI',
    mistral: 'Mistral',
  };

  function providerColor(provider: string): string {
    return PROVIDER_COLORS[provider.toLowerCase()] ?? PROVIDER_DEFAULT;
  }

  function providerLabel(provider: string): string {
    return PROVIDER_LABELS[provider.toLowerCase()] ?? provider;
  }

  interface ModelStatsRow {
    model_call_count: number;
    total_input_tokens: number;
    total_output_tokens: number;
    total_model_duration_ms: number;
    total_estimated_cost_usd: number;
  }

  let providerUsage = $state<ProviderTokenUsage[]>([]);
  let toolUsage = $state<ToolUsageCount[]>([]);
  let modelStats = $state<ModelStatsRow | null>(null);
  let traces = $state<TraceSummary[]>([]);
  let sessions = $state<SessionRecord[]>([]);
  let error = $state<string | null>(null);

  // Canvas refs
  let usageCanvas = $state<HTMLCanvasElement | undefined>();
  let costDoughnutCanvas = $state<HTMLCanvasElement | undefined>();
  let sessionsTimeCanvas = $state<HTMLCanvasElement | undefined>();
  let tokensTimeCanvas = $state<HTMLCanvasElement | undefined>();
  let costTimeCanvas = $state<HTMLCanvasElement | undefined>();
  let modelUsageCanvas = $state<HTMLCanvasElement | undefined>();
  let modelCostCanvas = $state<HTMLCanvasElement | undefined>();

  // Chart instances
  let usageChart: Chart | null = null;
  let costDoughnutChart: Chart | null = null;
  let sessionsTimeChart: Chart | null = null;
  let tokensTimeChart: Chart | null = null;
  let costTimeChart: Chart | null = null;
  let modelUsageChart: Chart | null = null;
  let modelCostChart: Chart | null = null;

  onMount(async () => {
    try {
      [providerUsage, toolUsage, modelStats, traces, sessions] = await Promise.all([
        queryAll<ProviderTokenUsage>(PROVIDER_USAGE_SQL),
        queryAll<ToolUsageCount>(TOOL_USAGE_SQL),
        queryOne<ModelStatsRow>(MODEL_STATS_SQL),
        getTraces(50),
        getSessionHistory(50),
      ]);
    } catch (e) {
      error = String(e);
    }
  });

  onDestroy(() => {
    usageChart?.destroy();
    costDoughnutChart?.destroy();
    sessionsTimeChart?.destroy();
    tokensTimeChart?.destroy();
    costTimeChart?.destroy();
    modelUsageChart?.destroy();
    modelCostChart?.destroy();
  });

  const gridColor = 'rgba(128,128,128,0.1)';
  const tickColor = 'rgba(128,128,128,0.6)';
  const tickFont = { size: 10 as const };
  const monoFont = { size: 10 as const, family: 'monospace' };

  function tokenTickCallback(v: string | number) {
    const n = Number(v);
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(n >= 10_000 ? 0 : 1)}K`;
    return String(n);
  }

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

  function costTickCallback(v: string | number) {
    const n = Number(v);
    if (n === 0) return '$0';
    if (n < 0.01) return `$${n.toFixed(3)}`;
    return `$${n.toFixed(2)}`;
  }

  function formatTokens2(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return `${n}`;
  }

  // Row 1: Usage per provider (vertical bar)
  $effect(() => {
    if (!usageCanvas || providerUsage.length === 0) return;
    if (usageChart) return;
    const providers = providerUsage;
    usageChart = new Chart(usageCanvas, {
      type: 'bar',
      data: {
        labels: providers.map(p => providerLabel(p.provider)),
        datasets: [{
          label: 'Tokens',
          data: providers.map(p => p.total_input_tokens + p.total_output_tokens),
          backgroundColor: providers.map(p => providerColor(p.provider)),
          borderRadius: 3, borderSkipped: false,
        }],
      },
      options: {
        responsive: true, maintainAspectRatio: false, animation: { duration: 400 },
        scales: {
          x: { grid: { display: false }, ticks: { color: tickColor, font: monoFont } },
          y: { beginAtZero: true, grid: { color: gridColor }, ticks: { color: tickColor, font: tickFont, callback: tokenTickCallback } },
        },
        plugins: { legend: { display: false }, tooltip: { callbacks: { label: (item) => `${formatTokens(Number(item.raw))} tokens` } } },
      },
    });
  });

  // Row 1: Cost doughnut
  $effect(() => {
    if (!costDoughnutCanvas || providerUsage.length === 0) return;
    if (costDoughnutChart) return;
    const providers = providerUsage.filter(p => p.total_estimated_cost_usd > 0);
    if (providers.length === 0) return;
    const totalCost = providers.reduce((s, p) => s + p.total_estimated_cost_usd, 0);
    costDoughnutChart = new Chart(costDoughnutCanvas, {
      type: 'doughnut',
      data: {
        labels: providers.map(p => providerLabel(p.provider)),
        datasets: [{
          data: providers.map(p => p.total_estimated_cost_usd),
          backgroundColor: providers.map(p => providerColor(p.provider)),
          borderWidth: 2, borderColor: 'rgba(0,0,0,0.1)',
        }],
      },
      options: {
        responsive: true, maintainAspectRatio: false, cutout: '60%', animation: { duration: 400 },
        plugins: {
          legend: { display: true, position: 'right', labels: { color: tickColor, font: tickFont, boxWidth: 10, boxHeight: 10, padding: 8 } },
          tooltip: {
            callbacks: {
              label: (item) => {
                const pct = totalCost > 0 ? ((Number(item.raw) / totalCost) * 100).toFixed(0) : '0';
                return `${item.label}: ${formatCost(Number(item.raw))} (${pct}%)`;
              },
            },
          },
        },
      },
    });
  });

  // Row 2: Sessions over time
  $effect(() => {
    if (!sessionsTimeCanvas || sessions.length === 0) return;
    if (sessionsTimeChart) return;
    // Bucket sessions by date
    const dateCounts = new Map<string, number>();
    for (const s of sessions) {
      const day = s.created_at.slice(0, 10);
      dateCounts.set(day, (dateCounts.get(day) ?? 0) + 1);
    }
    const sorted = Array.from(dateCounts.entries()).sort((a, b) => a[0].localeCompare(b[0]));
    sessionsTimeChart = new Chart(sessionsTimeCanvas, {
      type: 'bar',
      data: {
        labels: sorted.map(([d]) => d),
        datasets: [{
          label: 'Sessions',
          data: sorted.map(([, c]) => c),
          backgroundColor: 'rgba(59, 130, 246, 0.85)',
          borderRadius: 3, borderSkipped: false,
        }],
      },
      options: {
        responsive: true, maintainAspectRatio: false, animation: { duration: 400 },
        scales: {
          x: { grid: { display: false }, ticks: { color: tickColor, font: monoFont } },
          y: { beginAtZero: true, grid: { color: gridColor }, ticks: { color: tickColor, font: tickFont, stepSize: 1 } },
        },
        plugins: { legend: { display: false }, tooltip: { callbacks: { label: (item) => `${item.raw} sessions` } } },
      },
    });
  });

  // Row 3: Tokens over time (from traces, bucketed by time)
  const traceBuckets = $derived.by(() => {
    if (traces.length === 0) return [];
    const sorted = [...traces].sort((a, b) => a.started_at - b.started_at);
    const bucketSize = 5; // group every 5 traces
    const buckets: { label: string; input: number; output: number }[] = [];
    for (let i = 0; i < sorted.length; i += bucketSize) {
      const slice = sorted.slice(i, i + bucketSize);
      const input = slice.reduce((s, t) => s + t.total_input_tokens, 0);
      const output = slice.reduce((s, t) => s + t.total_output_tokens, 0);
      buckets.push({ label: `#${Math.floor(i / bucketSize) + 1}`, input, output });
    }
    return buckets;
  });

  $effect(() => {
    if (!tokensTimeCanvas || traceBuckets.length === 0) return;
    if (tokensTimeChart) return;
    tokensTimeChart = new Chart(tokensTimeCanvas, {
      type: 'bar',
      data: {
        labels: traceBuckets.map(b => b.label),
        datasets: [
          { label: 'Input', data: traceBuckets.map(b => b.input), backgroundColor: 'rgba(59, 130, 246, 0.85)', borderRadius: 2, borderSkipped: false },
          { label: 'Output', data: traceBuckets.map(b => b.output), backgroundColor: 'rgba(147, 197, 253, 0.7)', borderRadius: 2, borderSkipped: false },
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
          tooltip: { callbacks: { label: (item) => `${item.dataset.label}: ${formatTokens(Number(item.raw))}` } },
        },
      },
    });
  });

  // Row 3: Cost over time (from traces, bucketed, colored by provider)
  const costBuckets = $derived.by(() => {
    if (traces.length === 0) return { labels: [] as string[], datasets: [] as { label: string; data: number[]; backgroundColor: string }[] };
    const sorted = [...traces].sort((a, b) => a.started_at - b.started_at);
    const bucketSize = 5;
    const providers = new Set<string>();
    for (const t of sorted) providers.add(t.provider);
    const provArr = Array.from(providers);
    const labels: string[] = [];
    const provData = new Map<string, number[]>();
    for (const p of provArr) provData.set(p, []);

    for (let i = 0; i < sorted.length; i += bucketSize) {
      const slice = sorted.slice(i, i + bucketSize);
      labels.push(`#${Math.floor(i / bucketSize) + 1}`);
      for (const p of provArr) {
        const cost = slice.filter(t => t.provider === p).reduce((s, t) => s + t.total_estimated_cost_usd, 0);
        provData.get(p)!.push(cost);
      }
    }

    return {
      labels,
      datasets: provArr.map(p => ({
        label: providerLabel(p),
        data: provData.get(p)!,
        backgroundColor: providerColor(p),
      })),
    };
  });

  $effect(() => {
    if (!costTimeCanvas || costBuckets.labels.length === 0) return;
    if (costTimeChart) return;
    costTimeChart = new Chart(costTimeCanvas, {
      type: 'bar',
      data: {
        labels: costBuckets.labels,
        datasets: costBuckets.datasets.map(ds => ({ ...ds, borderRadius: 2, borderSkipped: false })),
      },
      options: {
        responsive: true, maintainAspectRatio: false, animation: { duration: 400 },
        scales: {
          x: { stacked: true, grid: { display: false }, ticks: { color: tickColor, font: monoFont } },
          y: { stacked: true, beginAtZero: true, grid: { color: gridColor }, ticks: { color: tickColor, font: tickFont, callback: costTickCallback } },
        },
        plugins: {
          legend: { display: true, position: 'bottom', labels: { color: tickColor, font: tickFont, boxWidth: 12, boxHeight: 10, padding: 12 } },
          tooltip: { callbacks: { label: (item) => `${item.dataset.label}: ${formatCost(Number(item.raw))}` } },
        },
      },
    });
  });

  // Row 4: Model usage (tokens) and model cost -- grouped by model, colored by provider
  const modelData = $derived.by(() => {
    if (traces.length === 0) return [];
    const models = new Map<string, { model: string; provider: string; tokens: number; cost: number }>();
    for (const t of traces) {
      const m = t.model ?? t.provider;
      const existing = models.get(m);
      if (existing) {
        existing.tokens += t.total_input_tokens + t.total_output_tokens;
        existing.cost += t.total_estimated_cost_usd;
      } else {
        models.set(m, { model: m, provider: t.provider, tokens: t.total_input_tokens + t.total_output_tokens, cost: t.total_estimated_cost_usd });
      }
    }
    return Array.from(models.values()).sort((a, b) => b.tokens - a.tokens);
  });

  $effect(() => {
    if (!modelUsageCanvas || modelData.length === 0) return;
    if (modelUsageChart) return;
    modelUsageChart = new Chart(modelUsageCanvas, {
      type: 'bar',
      data: {
        labels: modelData.map(m => m.model),
        datasets: [{
          label: 'Tokens',
          data: modelData.map(m => m.tokens),
          backgroundColor: modelData.map(m => providerColor(m.provider)),
          borderRadius: 3, borderSkipped: false,
        }],
      },
      options: {
        responsive: true, maintainAspectRatio: false, animation: { duration: 400 },
        scales: {
          x: { grid: { display: false }, ticks: { color: tickColor, font: monoFont, maxRotation: 45, minRotation: 0 } },
          y: { beginAtZero: true, grid: { color: gridColor }, ticks: { color: tickColor, font: tickFont, callback: tokenTickCallback } },
        },
        plugins: { legend: { display: false }, tooltip: { callbacks: { label: (item) => `${formatTokens(Number(item.raw))} tokens` } } },
      },
    });
  });

  $effect(() => {
    if (!modelCostCanvas || modelData.length === 0) return;
    if (modelCostChart) return;
    modelCostChart = new Chart(modelCostCanvas, {
      type: 'bar',
      data: {
        labels: modelData.map(m => m.model),
        datasets: [{
          label: 'Cost',
          data: modelData.map(m => m.cost),
          backgroundColor: modelData.map(m => providerColor(m.provider)),
          borderRadius: 3, borderSkipped: false,
        }],
      },
      options: {
        responsive: true, maintainAspectRatio: false, animation: { duration: 400 },
        scales: {
          x: { grid: { display: false }, ticks: { color: tickColor, font: monoFont, maxRotation: 45, minRotation: 0 } },
          y: { beginAtZero: true, grid: { color: gridColor }, ticks: { color: tickColor, font: tickFont, callback: costTickCallback } },
        },
        plugins: { legend: { display: false }, tooltip: { callbacks: { label: (item) => formatCost(Number(item.raw)) } } },
      },
    });
  });
</script>

<div class="space-y-6">
  {#if modelStats}
    <!-- Row 1: Usage per provider, cost doughnut, avg tokens -->
    <div class="grid grid-cols-3 gap-4">
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <h4 class="text-xs font-semibold text-base-content/60 mb-2">Usage per provider</h4>
        {#if providerUsage.length > 0}
          <div class="h-48"><canvas bind:this={usageCanvas}></canvas></div>
        {:else}
          <div class="flex items-center justify-center h-48 text-[10px] text-base-content/40">No provider data</div>
        {/if}
      </div>
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <h4 class="text-xs font-semibold text-base-content/60 mb-2">Cost per provider</h4>
        {#if providerUsage.length > 0}
          <div class="h-48 relative">
            <canvas bind:this={costDoughnutCanvas}></canvas>
            <div class="absolute inset-0 flex items-center justify-center pointer-events-none" style:padding-right="60px">
              <div class="text-center">
                <div class="text-lg font-semibold tabular-nums text-info">{formatCost(modelStats?.total_estimated_cost_usd ?? 0)}</div>
                <div class="text-[10px] text-base-content/40">total</div>
              </div>
            </div>
          </div>
        {:else}
          <div class="flex items-center justify-center h-48 text-[10px] text-base-content/40">No cost data</div>
        {/if}
      </div>
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3 flex flex-col items-center justify-center">
        <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Total Tokens</div>
        <div class="mt-2 text-3xl font-semibold tabular-nums">{formatTokens2((modelStats?.total_input_tokens ?? 0) + (modelStats?.total_output_tokens ?? 0))}</div>
        <div class="mt-1 flex gap-2 text-[10px] text-base-content/40">
          <span>{formatTokens(modelStats?.total_input_tokens ?? 0)} in</span>
          <span>{formatTokens(modelStats?.total_output_tokens ?? 0)} out</span>
        </div>
      </div>
    </div>

    <!-- Row 2: Sessions over time -->
    {#if sessions.length > 0}
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <h4 class="text-xs font-semibold text-base-content/60 mb-2">Sessions over time</h4>
        <div class="h-40"><canvas bind:this={sessionsTimeCanvas}></canvas></div>
      </div>
    {/if}

    <!-- Row 3: Tokens over time + Cost over time -->
    {#if traces.length > 0}
      <div class="grid grid-cols-2 gap-4">
        <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
          <h4 class="text-xs font-semibold text-base-content/60 mb-2">Tokens over time</h4>
          <div class="h-48"><canvas bind:this={tokensTimeCanvas}></canvas></div>
        </div>
        <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
          <h4 class="text-xs font-semibold text-base-content/60 mb-2">Cost over time</h4>
          <div class="h-48"><canvas bind:this={costTimeCanvas}></canvas></div>
        </div>
      </div>
    {/if}

    <!-- Row 4: Model usage (tokens) + Model cost -->
    {#if modelData.length > 0}
      <div class="grid grid-cols-2 gap-4">
        <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
          <h4 class="text-xs font-semibold text-base-content/60 mb-2">Model usage (tokens)</h4>
          <div class="h-48"><canvas bind:this={modelUsageCanvas}></canvas></div>
        </div>
        <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
          <h4 class="text-xs font-semibold text-base-content/60 mb-2">Model cost</h4>
          <div class="h-48"><canvas bind:this={modelCostCanvas}></canvas></div>
        </div>
      </div>
    {/if}
  {:else if error}
    <div class="flex items-center justify-center h-32 text-base-content/40 text-sm">{error}</div>
  {:else}
    <div class="flex items-center justify-center h-32">
      <span class="loading loading-spinner loading-md"></span>
    </div>
  {/if}
</div>
