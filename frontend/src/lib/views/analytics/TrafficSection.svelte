<script lang="ts">
  import { onDestroy } from 'svelte';
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
  import { networkStore } from '../../stores/network.svelte';

  Chart.register(BarController, BarElement, CategoryScale, LinearScale, DoughnutController, ArcElement, Tooltip, Legend);

  let timeCanvas = $state<HTMLCanvasElement | undefined>();
  let domainCanvas = $state<HTMLCanvasElement | undefined>();
  let methodCanvas = $state<HTMLCanvasElement | undefined>();
  let processCanvas = $state<HTMLCanvasElement | undefined>();

  let timeChart: Chart | null = null;
  let domainChart: Chart | null = null;
  let methodChart: Chart | null = null;
  let processChart: Chart | null = null;

  onDestroy(() => {
    timeChart?.destroy();
    domainChart?.destroy();
    methodChart?.destroy();
    processChart?.destroy();
  });

  const gridColor = 'rgba(128,128,128,0.1)';
  const tickColor = 'rgba(128,128,128,0.6)';
  const tickFont = { size: 10 as const };
  const monoFont = { size: 10 as const, family: 'monospace' };
  const BLUE = 'rgba(59, 130, 246, 0.85)';
  const PURPLE = 'rgba(139, 92, 246, 0.85)';
  const METHOD_COLORS = [BLUE, PURPLE, 'rgba(14, 165, 233, 0.85)', 'rgba(99, 102, 241, 0.85)', 'rgba(168, 85, 247, 0.85)', 'rgba(6, 182, 212, 0.85)'];

  // Requests over time (from SQL)
  const timeBuckets = $derived(networkStore.timeBuckets);

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

  // HTTP methods doughnut (from SQL)
  const methodCounts = $derived(networkStore.methodDist);

  $effect(() => {
    if (!methodCanvas || methodCounts.length === 0) return;
    const labels = methodCounts.map(m => m.method);
    const data = methodCounts.map(m => m.count);
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
        datasets: [{ data, backgroundColor: labels.map((_, i) => METHOD_COLORS[i % METHOD_COLORS.length]), borderWidth: 2, borderColor: 'rgba(0,0,0,0.1)' }],
      },
      options: {
        responsive: true, maintainAspectRatio: false, cutout: '50%', animation: { duration: 400 },
        plugins: {
          legend: { display: true, position: 'right', labels: { color: tickColor, font: tickFont, boxWidth: 10, boxHeight: 10, padding: 8 } },
          tooltip: { callbacks: { label: (item) => { const total = methodCounts.reduce((s, m) => s + m.count, 0); const pct = total > 0 ? ((Number(item.raw) / total) * 100).toFixed(0) : '0'; return `${item.label}: ${item.raw} (${pct}%)`; } } },
        },
      },
    });
  });

  // Domains bar chart (from SQL)
  const domainCounts = $derived(networkStore.topDomains);

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
        plugins: { legend: { display: false }, tooltip: { callbacks: { title: (items) => domainCounts[items[0].dataIndex]?.domain ?? '', label: (item) => `${item.dataset.label}: ${item.raw}` } } },
      },
    });
  });

  // Top processes (from SQL)
  const processCounts = $derived(networkStore.processDist);

  $effect(() => {
    if (!processCanvas || processCounts.length === 0) return;
    const labels = processCounts.map(p => p.process_name);
    const data = processCounts.map(p => p.count);
    if (processChart) {
      processChart.data.labels = labels;
      processChart.data.datasets[0].data = data;
      processChart.update('none');
      return;
    }
    processChart = new Chart(processCanvas, {
      type: 'bar',
      data: {
        labels,
        datasets: [{
          label: 'Requests',
          data,
          backgroundColor: BLUE,
          borderRadius: 3, borderSkipped: false,
        }],
      },
      options: {
        responsive: true, maintainAspectRatio: false, animation: { duration: 400 },
        scales: {
          x: { grid: { display: false }, ticks: { color: tickColor, font: monoFont } },
          y: { beginAtZero: true, grid: { color: gridColor }, ticks: { color: tickColor, font: tickFont, stepSize: 1 } },
        },
        plugins: { legend: { display: false }, tooltip: { callbacks: { label: (item) => `${item.raw} requests` } } },
      },
    });
  });
</script>

<div class="space-y-6">
  {#if networkStore.totalCalls > 0}
    <!-- Top stat cards -->
    <div class="grid grid-cols-4 gap-3">
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Total Requests</div>
        <div class="mt-1 text-xl font-semibold tabular-nums">{networkStore.totalCalls}</div>
      </div>
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Accepted</div>
        <div class="mt-1 text-xl font-semibold tabular-nums text-info">{networkStore.allowedCount}</div>
      </div>
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Denied</div>
        <div class="mt-1 text-xl font-semibold tabular-nums text-secondary">{networkStore.deniedCount}</div>
      </div>
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Avg Latency</div>
        <div class="mt-1 text-xl font-semibold tabular-nums">{networkStore.avgLatency}ms</div>
      </div>
    </div>

    <!-- Row 2: Requests over time + HTTP methods -->
    <div class="grid grid-cols-2 gap-4">
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <h4 class="text-xs font-semibold text-base-content/60 mb-2">Requests over time</h4>
        <div class="h-48"><canvas bind:this={timeCanvas}></canvas></div>
      </div>
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <h4 class="text-xs font-semibold text-base-content/60 mb-2">HTTP Methods</h4>
        {#if methodCounts.length > 0}
          <div class="h-48"><canvas bind:this={methodCanvas}></canvas></div>
        {:else}
          <div class="flex items-center justify-center h-48 text-[10px] text-base-content/40">No method data</div>
        {/if}
      </div>
    </div>

    <!-- Row 3: Domains + Processes -->
    <div class="grid grid-cols-2 gap-4">
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <h4 class="text-xs font-semibold text-base-content/60 mb-2">Top domains</h4>
        {#if domainCounts.length > 0}
          <div class="h-48"><canvas bind:this={domainCanvas}></canvas></div>
        {:else}
          <div class="flex items-center justify-center h-48 text-[10px] text-base-content/40">No domain data</div>
        {/if}
      </div>
      <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
        <h4 class="text-xs font-semibold text-base-content/60 mb-2">Top processes</h4>
        {#if processCounts.length > 0}
          <div class="h-48"><canvas bind:this={processCanvas}></canvas></div>
        {:else}
          <div class="flex items-center justify-center h-48 text-[10px] text-base-content/40">No process data</div>
        {/if}
      </div>
    </div>
  {:else}
    <div class="flex items-center justify-center rounded-lg border border-base-300 bg-base-200/50 py-8">
      <span class="text-sm text-base-content/40">No network events recorded</span>
    </div>
  {/if}
</div>
