<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import {
    Chart,
    BarController,
    BarElement,
    DoughnutController,
    ArcElement,
    CategoryScale,
    LinearScale,
    Tooltip,
    Legend,
  } from 'chart.js';
  import { queryOne, queryAll } from '../../db';
  import { FILE_STATS_SQL, FILE_EVENTS_SQL, FILE_EVENTS_SEARCH_SQL } from '../../sql';
  import type { FileEvent } from '../../types';

  Chart.register(BarController, BarElement, DoughnutController, ArcElement, CategoryScale, LinearScale, Tooltip, Legend);

  interface FileStatsRow {
    total: number;
    created: number;
    modified: number;
    deleted: number;
  }

  let stats = $state<FileStatsRow | null>(null);
  let events = $state<FileEvent[]>([]);
  let search = $state('');
  let searchResults = $state<FileEvent[] | null>(null);
  let pollTimer: ReturnType<typeof setInterval> | null = null;
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  // Canvas refs
  let actionCanvas = $state<HTMLCanvasElement | undefined>();
  let timeCanvas = $state<HTMLCanvasElement | undefined>();

  // Chart instances
  let actionChart: Chart | null = null;
  let timeChart: Chart | null = null;

  // Color palette (blue/sky/purple -- no green/red)
  const BLUE = 'oklch(0.7 0.15 250)';
  const SKY = 'oklch(0.75 0.12 220)';
  const PURPLE = 'oklch(0.65 0.15 300)';

  async function refresh() {
    try {
      const [s, e] = await Promise.all([
        queryOne<FileStatsRow>(FILE_STATS_SQL),
        queryAll<FileEvent>(FILE_EVENTS_SQL, [200]),
      ]);
      stats = s;
      events = e;
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
    if (debounceTimer) clearTimeout(debounceTimer);
    actionChart?.destroy();
    timeChart?.destroy();
  });

  function doSearch(query: string) {
    if (debounceTimer) clearTimeout(debounceTimer);
    if (!query.trim()) {
      searchResults = null;
      return;
    }
    debounceTimer = setTimeout(async () => {
      try {
        searchResults = await queryAll<FileEvent>(FILE_EVENTS_SEARCH_SQL, [query, 100]);
      } catch {
        searchResults = [];
      }
    }, 300);
  }

  $effect(() => {
    doSearch(search);
  });

  const displayEvents = $derived(searchResults ?? events.slice(0, 100));

  function actionBadge(action: string): string {
    switch (action) {
      case 'created': return 'badge-info';
      case 'modified': return 'badge-ghost';
      case 'deleted': return 'badge-secondary';
      default: return 'badge-ghost';
    }
  }

  function timeAgo(epoch: number): string {
    const diff = Math.floor(Date.now() / 1000) - epoch;
    if (diff < 60) return `${diff}s ago`;
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    return `${Math.floor(diff / 86400)}d ago`;
  }

  function formatBytes(n: number | null): string {
    if (n === null || n === 0) return '--';
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  }

  const gridColor = 'rgba(128,128,128,0.1)';
  const tickColor = 'rgba(128,128,128,0.6)';
  const tickFont = { size: 10 as const };
  const monoFont = { size: 10 as const, family: 'monospace' };

  // Action breakdown doughnut
  $effect(() => {
    if (!actionCanvas || !stats || stats.total === 0) return;
    if (actionChart) {
      actionChart.data.datasets[0].data = [stats.created, stats.modified, stats.deleted];
      actionChart.update('none');
      return;
    }
    actionChart = new Chart(actionCanvas, {
      type: 'doughnut',
      data: {
        labels: ['Created', 'Modified', 'Deleted'],
        datasets: [{
          data: [stats.created, stats.modified, stats.deleted],
          backgroundColor: [BLUE, SKY, PURPLE],
          borderWidth: 2, borderColor: 'rgba(0,0,0,0.1)',
        }],
      },
      options: {
        responsive: true, maintainAspectRatio: false, cutout: '60%', animation: { duration: 400 },
        plugins: {
          legend: { display: true, position: 'right', labels: { color: tickColor, font: tickFont, boxWidth: 10, boxHeight: 10, padding: 8 } },
          tooltip: { callbacks: { label: (item) => `${item.label}: ${item.raw}` } },
        },
      },
    });
  });

  // Events over time (stacked bar, bucketed)
  const timeBuckets = $derived.by(() => {
    if (events.length === 0) return { labels: [] as string[], created: [] as number[], modified: [] as number[], deleted: [] as number[] };
    const sorted = [...events].sort((a, b) => a.timestamp - b.timestamp);
    const bucketSize = 10;
    const labels: string[] = [];
    const created: number[] = [];
    const modified: number[] = [];
    const deleted: number[] = [];
    for (let i = 0; i < sorted.length; i += bucketSize) {
      const slice = sorted.slice(i, i + bucketSize);
      labels.push(`#${Math.floor(i / bucketSize) + 1}`);
      created.push(slice.filter(e => e.action === 'created').length);
      modified.push(slice.filter(e => e.action === 'modified').length);
      deleted.push(slice.filter(e => e.action === 'deleted').length);
    }
    return { labels, created, modified, deleted };
  });

  $effect(() => {
    if (!timeCanvas || timeBuckets.labels.length === 0) return;
    if (timeChart) {
      timeChart.data.labels = timeBuckets.labels;
      timeChart.data.datasets[0].data = timeBuckets.created;
      timeChart.data.datasets[1].data = timeBuckets.modified;
      timeChart.data.datasets[2].data = timeBuckets.deleted;
      timeChart.update('none');
      return;
    }
    timeChart = new Chart(timeCanvas, {
      type: 'bar',
      data: {
        labels: timeBuckets.labels,
        datasets: [
          { label: 'Created', data: timeBuckets.created, backgroundColor: BLUE, borderRadius: 2, borderSkipped: false },
          { label: 'Modified', data: timeBuckets.modified, backgroundColor: SKY, borderRadius: 2, borderSkipped: false },
          { label: 'Deleted', data: timeBuckets.deleted, backgroundColor: PURPLE, borderRadius: 2, borderSkipped: false },
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
</script>

<div class="space-y-6">
  <!-- Stat cards -->
  <div class="grid grid-cols-4 gap-3">
    <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
      <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Total</div>
      <div class="mt-1 text-xl font-semibold tabular-nums">{stats?.total ?? 0}</div>
    </div>
    <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
      <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Created</div>
      <div class="mt-1 text-xl font-semibold tabular-nums text-info">{stats?.created ?? 0}</div>
    </div>
    <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
      <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Modified</div>
      <div class="mt-1 text-xl font-semibold tabular-nums">{stats?.modified ?? 0}</div>
    </div>
    <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
      <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Deleted</div>
      <div class="mt-1 text-xl font-semibold tabular-nums text-secondary">{stats?.deleted ?? 0}</div>
    </div>
  </div>

  <!-- Charts row -->
  {#if stats && stats.total > 0}
    <div class="grid grid-cols-2 gap-4">
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <h4 class="text-xs font-semibold text-base-content/60 mb-2">Action breakdown</h4>
        <div class="h-48"><canvas bind:this={actionCanvas}></canvas></div>
      </div>
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <h4 class="text-xs font-semibold text-base-content/60 mb-2">Events over time</h4>
        <div class="h-48"><canvas bind:this={timeCanvas}></canvas></div>
      </div>
    </div>
  {/if}

  <!-- Search -->
  <input
    type="text"
    class="input input-sm input-bordered w-full font-mono text-xs"
    placeholder="Search file paths..."
    bind:value={search}
  />

  <!-- Event table -->
  {#if displayEvents.length > 0}
    <div class="overflow-x-auto rounded-lg border border-base-300">
      <table class="table table-xs w-full">
        <thead>
          <tr class="text-base-content/50">
            <th class="font-medium">Time</th>
            <th class="font-medium">Action</th>
            <th class="font-medium">Path</th>
            <th class="font-medium text-right">Size</th>
          </tr>
        </thead>
        <tbody>
          {#each displayEvents as ev}
            <tr class="hover">
              <td class="font-mono text-[10px] text-base-content/40 whitespace-nowrap">{timeAgo(ev.timestamp)}</td>
              <td><span class="badge badge-xs {actionBadge(ev.action)}">{ev.action}</span></td>
              <td class="font-mono text-xs truncate max-w-[300px]" title={ev.path}>{ev.path}</td>
              <td class="font-mono text-[10px] text-base-content/40 text-right">{formatBytes(ev.size)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else if search.trim()}
    <div class="flex items-center justify-center rounded-lg border border-base-300 bg-base-200/50 py-8">
      <span class="text-sm text-base-content/40">No matching file events</span>
    </div>
  {:else}
    <div class="flex items-center justify-center rounded-lg border border-base-300 bg-base-200/50 py-8">
      <span class="text-sm text-base-content/40">No file events recorded yet</span>
    </div>
  {/if}
</div>
