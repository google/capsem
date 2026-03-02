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
  import { GLOBAL_STATS_SQL, TOP_PROVIDERS_SQL, TOP_TOOLS_SQL, SESSION_HISTORY_SQL } from '../../sql';
  import type { GlobalStats, ProviderSummary, ToolSummary, SessionRecord } from '../../types';

  Chart.register(BarController, BarElement, CategoryScale, LinearScale, Tooltip, Legend);

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

  let globalStats = $state<GlobalStats | null>(null);
  let providers = $state<ProviderSummary[]>([]);
  let topTools = $state<ToolSummary[]>([]);
  let sessions = $state<SessionRecord[]>([]);
  let error = $state<string | null>(null);

  // Tool chart
  let toolCanvas = $state<HTMLCanvasElement | undefined>();
  let toolChart: Chart | null = null;

  // Session detail expansion
  let selectedSession = $state<SessionRecord | null>(null);

  onMount(async () => {
    try {
      [globalStats, providers, topTools, sessions] = await Promise.all([
        queryOne<GlobalStats>(GLOBAL_STATS_SQL, [], 'main'),
        queryAll<ProviderSummary>(TOP_PROVIDERS_SQL, [5], 'main'),
        queryAll<ToolSummary>(TOP_TOOLS_SQL, [5], 'main'),
        queryAll<SessionRecord>(SESSION_HISTORY_SQL, [50], 'main'),
      ]);
    } catch (e) {
      error = String(e);
    }
  });

  onDestroy(() => {
    toolChart?.destroy();
  });

  const gridColor = 'rgba(128,128,128,0.1)';
  const tickColor = 'rgba(128,128,128,0.6)';
  const tickFont = { size: 10 as const };
  const monoFont = { size: 10 as const, family: 'monospace' };

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

  function formatTime(iso: string): string {
    return new Date(iso).toLocaleString();
  }

  function formatDuration(start: string, end: string | null): string {
    if (!end) return 'running';
    const ms = new Date(end).getTime() - new Date(start).getTime();
    const mins = Math.floor(ms / 60000);
    if (mins < 60) return `${mins}m`;
    const hours = Math.floor(mins / 60);
    return `${hours}h ${mins % 60}m`;
  }

  function formatBytes(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)} MB`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)} KB`;
    return `${n} B`;
  }

  function statusBadge(status: string): string {
    switch (status) {
      case 'running': return 'badge-info';
      case 'stopped': return 'badge-ghost';
      case 'crashed': return 'badge-secondary';
      case 'vacuumed': return 'badge-ghost';
      case 'terminated': return 'badge-ghost opacity-50';
      default: return 'badge-ghost';
    }
  }

  $effect(() => {
    if (!toolCanvas || topTools.length === 0) return;
    const labels = topTools.map(t => t.tool_name);
    const data = topTools.map(t => t.call_count);
    if (toolChart) {
      toolChart.data.labels = labels;
      toolChart.data.datasets[0].data = data;
      toolChart.update('none');
      return;
    }
    toolChart = new Chart(toolCanvas, {
      type: 'bar',
      data: {
        labels,
        datasets: [{ label: 'Calls', data, backgroundColor: 'rgba(59, 130, 246, 0.85)', borderRadius: 3, borderSkipped: false }],
      },
      options: {
        indexAxis: 'y',
        responsive: true, maintainAspectRatio: false, animation: { duration: 400 },
        scales: {
          x: { beginAtZero: true, grid: { color: gridColor }, ticks: { color: tickColor, font: tickFont, stepSize: 1 } },
          y: { grid: { display: false }, ticks: { color: tickColor, font: monoFont } },
        },
        plugins: {
          legend: { display: false },
          tooltip: {
            callbacks: {
              label: (item) => {
                const tool = topTools[item.dataIndex];
                const avgMs = tool.call_count > 0 ? Math.round(tool.total_duration_ms / tool.call_count) : 0;
                return `${item.raw} calls (avg ${avgMs}ms)`;
              },
            },
          },
        },
      },
    });
  });
</script>

<div class="space-y-6">
  {#if globalStats}
    <!-- Top stat cards -->
    <div class="grid grid-cols-4 gap-3">
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Sessions</div>
        <div class="mt-1 text-xl font-semibold tabular-nums">{globalStats.total_sessions}</div>
      </div>
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Total Cost</div>
        <div class="mt-1 text-xl font-semibold tabular-nums text-info">{formatCost(globalStats.total_estimated_cost)}</div>
      </div>
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Total Tokens</div>
        <div class="mt-1 text-xl font-semibold tabular-nums">{formatTokens(globalStats.total_input_tokens + globalStats.total_output_tokens)}</div>
        <div class="mt-1 flex gap-2 text-[10px] text-base-content/50">
          <span>{formatTokens(globalStats.total_input_tokens)} in</span>
          <span>{formatTokens(globalStats.total_output_tokens)} out</span>
        </div>
      </div>
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Tool Calls</div>
        <div class="mt-1 text-xl font-semibold tabular-nums">{globalStats.total_tool_calls}</div>
      </div>
    </div>

    <!-- Middle row: provider usage + top tools -->
    <div class="grid grid-cols-2 gap-4">
      <!-- Provider usage -->
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <h4 class="text-xs font-semibold text-base-content/60 mb-2">Provider usage</h4>
        {#if providers.length > 0}
          <div class="space-y-2">
            {#each providers as pu}
              {@const maxTokens = Math.max(...providers.map(p => p.input_tokens + p.output_tokens))}
              {@const tokens = pu.input_tokens + pu.output_tokens}
              {@const pct = maxTokens > 0 ? (tokens / maxTokens) * 100 : 0}
              <div class="space-y-0.5">
                <div class="flex items-center justify-between text-xs">
                  <span class="font-mono">{providerLabel(pu.provider)}</span>
                  <div class="flex gap-3 text-base-content/50 tabular-nums">
                    <span>{formatTokens(tokens)} tokens</span>
                    <span>{pu.call_count} calls</span>
                    <span class="text-info">{formatCost(pu.estimated_cost)}</span>
                  </div>
                </div>
                <div class="h-1.5 rounded-full bg-base-300/50 overflow-hidden">
                  <div
                    class="h-full rounded-full transition-all"
                    style:width="{pct}%"
                    style:background-color={providerColor(pu.provider)}
                  ></div>
                </div>
              </div>
            {/each}
          </div>
        {:else}
          <div class="flex items-center justify-center h-32 text-[10px] text-base-content/40">No provider data</div>
        {/if}
      </div>

      <!-- Top 5 tools -->
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <h4 class="text-xs font-semibold text-base-content/60 mb-2">Top tools</h4>
        {#if topTools.length > 0}
          <div class="h-48"><canvas bind:this={toolCanvas}></canvas></div>
        {:else}
          <div class="flex items-center justify-center h-48 text-[10px] text-base-content/40">No tool calls recorded</div>
        {/if}
      </div>
    </div>

    <!-- Session history table -->
    {#if selectedSession}
      <!-- Session detail -->
      <div>
        <button
          class="flex items-center gap-1 text-xs text-base-content/60 hover:text-base-content mb-3"
          onclick={() => selectedSession = null}
        >
          <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor" class="size-4">
            <path d="M20 11H7.83l5.59-5.59L12 4l-8 8 8 8 1.41-1.41L7.83 13H20v-2z"/>
          </svg>
          <span>Back to sessions</span>
        </button>

        <div class="flex items-center gap-3 mb-4">
          <h3 class="text-sm font-semibold font-mono">{selectedSession.id}</h3>
          <span class="badge badge-xs {statusBadge(selectedSession.status)}">{selectedSession.status}</span>
          {#if selectedSession.mode === 'cli' && selectedSession.command}
            <span class="text-xs text-base-content/50 font-mono">{selectedSession.command}</span>
          {/if}
        </div>

        <div class="grid grid-cols-4 gap-3">
          <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
            <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Duration</div>
            <div class="mt-1 text-xl font-semibold">{formatDuration(selectedSession.created_at, selectedSession.stopped_at)}</div>
            <div class="mt-1 text-[10px] text-base-content/50">{formatTime(selectedSession.created_at)}</div>
          </div>
          <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
            <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Tokens</div>
            <div class="mt-1 text-xl font-semibold tabular-nums">{formatTokens(selectedSession.total_input_tokens + selectedSession.total_output_tokens)}</div>
            <div class="mt-1 flex gap-2 text-[10px] text-base-content/50">
              <span>{formatTokens(selectedSession.total_input_tokens)} in</span>
              <span>{formatTokens(selectedSession.total_output_tokens)} out</span>
            </div>
          </div>
          <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
            <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Cost</div>
            <div class="mt-1 text-xl font-semibold tabular-nums text-info">{formatCost(selectedSession.total_estimated_cost)}</div>
          </div>
          <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
            <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Tool Calls</div>
            <div class="mt-1 text-xl font-semibold tabular-nums">{selectedSession.total_tool_calls}</div>
            <div class="mt-1 text-[10px] text-base-content/50">{selectedSession.total_mcp_calls} MCP</div>
          </div>
        </div>

        <div class="grid grid-cols-3 gap-3 mt-3">
          <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
            <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">HTTPS Requests</div>
            <div class="mt-1 text-xl font-semibold tabular-nums">{selectedSession.total_requests}</div>
            <div class="mt-1 flex gap-2 text-[10px]">
              <span class="text-info">{selectedSession.allowed_requests} ok</span>
              <span class="text-secondary">{selectedSession.denied_requests} denied</span>
            </div>
          </div>
          <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
            <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Mode</div>
            <div class="mt-1 text-xl font-semibold">{selectedSession.mode}</div>
          </div>
          <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
            <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Disk</div>
            <div class="mt-1 text-xl font-semibold">{selectedSession.scratch_disk_size_gb} GB</div>
            <div class="mt-1 text-[10px] text-base-content/50">{formatBytes(selectedSession.ram_bytes)} RAM</div>
          </div>
        </div>
      </div>
    {:else}
      <div>
        <h3 class="text-sm font-semibold mb-3">Sessions</h3>
        {#if sessions.length === 0}
          <div class="text-sm text-base-content/40 py-8 text-center">No sessions recorded</div>
        {:else}
          <div class="rounded-lg border border-base-300 overflow-hidden">
            <table class="table table-zebra table-xs w-full">
              <thead class="bg-base-200">
                <tr>
                  <th>Session</th>
                  <th>Mode</th>
                  <th>Status</th>
                  <th>Started</th>
                  <th>Duration</th>
                  <th class="text-right">Tokens</th>
                  <th class="text-right">Cost</th>
                  <th class="text-right">Requests</th>
                </tr>
              </thead>
              <tbody>
                {#each sessions as session}
                  <tr
                    class="cursor-pointer hover:bg-base-200/80"
                    onclick={() => selectedSession = session}
                  >
                    <td class="font-mono text-xs">{session.id}</td>
                    <td>{session.mode}</td>
                    <td><span class="badge badge-xs {statusBadge(session.status)}">{session.status}</span></td>
                    <td class="font-mono text-xs">{formatTime(session.created_at)}</td>
                    <td class="tabular-nums">{formatDuration(session.created_at, session.stopped_at)}</td>
                    <td class="text-right tabular-nums">{formatTokens(session.total_input_tokens + session.total_output_tokens)}</td>
                    <td class="text-right tabular-nums text-info">{formatCost(session.total_estimated_cost)}</td>
                    <td class="text-right tabular-nums">{session.total_requests}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
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
