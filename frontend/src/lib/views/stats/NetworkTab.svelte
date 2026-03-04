<script lang="ts">
  import { onMount } from 'svelte';
  import { queryDb, queryAll, queryOne } from '../../db';
  import {
    NET_STATS_SQL, NET_REQUESTS_OVER_TIME_SQL, NET_METHODS_SQL,
    NET_TOP_DOMAINS_SQL, NET_EVENTS_ALL_SQL, NET_EVENTS_SEARCH_SQL,
  } from '../../sql';
  import { colors } from '../../css-var';
  import { BarChart, PieChart } from 'layerchart';
  import type { DetailSelection } from '../../types';
  import DetailPanel from './DetailPanel.svelte';
  import StatCards from './StatCards.svelte';

  let stats = $state<{ total: number; allowed: number; denied: number; avg_latency: number } | null>(null);
  let rows = $state<Record<string, unknown>[]>([]);
  let search = $state('');
  let searchDebounced = $state('');
  let detail = $state<DetailSelection | null>(null);
  let loaded = $state(false);
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  // Chart data
  let timeData = $state<{ bucket: string; allowed: number; denied: number }[]>([]);
  let methodSeries = $state<{ key: string; color: string; data: { key: string; value: number }[] }[]>([]);
  let domainData = $state<{ domain: string; allowed: number; denied: number }[]>([]);

  onMount(() => { loadAll(); });

  function onSearch(value: string) {
    search = value;
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => { searchDebounced = value; loadEvents(); }, 300);
  }

  async function loadAll() {
    await Promise.all([loadStats(), loadCharts(), loadEvents()]);
  }

  async function loadStats() {
    try {
      stats = queryOne(await queryDb(NET_STATS_SQL));
    } catch (e) { console.error('Net stats failed:', e); }
  }

  async function loadCharts() {
    try {
      const [timeRes, methodRes, domainRes] = await Promise.all([
        queryDb(NET_REQUESTS_OVER_TIME_SQL),
        queryDb(NET_METHODS_SQL),
        queryDb(NET_TOP_DOMAINS_SQL),
      ]);
      const timeRows = queryAll<{ bucket: number; allowed: number; denied: number }>(timeRes);
      timeData = timeRows.map((r, i) => ({ bucket: String(i + 1), allowed: r.allowed, denied: r.denied }));

      const methodRows = queryAll<{ method: string; cnt: number }>(methodRes);
      const mPalette = [allowedColor, deniedColor, colors.caution, colors.providerOpenai, colors.providerMistral];
      methodSeries = methodRows.map((r, i) => ({
        key: r.method,
        color: mPalette[i % mPalette.length],
        data: [{ key: r.method, value: r.cnt }],
      }));

      const domRows = queryAll<{ domain: string; allowed: number; denied: number }>(domainRes);
      domainData = domRows;
    } catch (e) { console.error('Net charts failed:', e); }
  }

  async function loadEvents() {
    loaded = false;
    try {
      const q = searchDebounced.trim();
      if (q) {
        const like = `%${q}%`;
        rows = queryAll(await queryDb(NET_EVENTS_SEARCH_SQL, [like, like, like]));
      } else {
        rows = queryAll(await queryDb(NET_EVENTS_ALL_SQL));
      }
    } catch (e) { console.error('Net events failed:', e); }
    loaded = true;
  }

  function selectRow(row: Record<string, unknown>) { detail = { type: 'net_event', data: row }; }

  function fmtTime(ts: unknown): string {
    if (!ts) return '';
    const s = String(ts);
    const idx = s.indexOf('T');
    return idx >= 0 ? s.substring(idx + 1, idx + 9) : s;
  }

  function fmtBytes(n: unknown): string {
    const v = Number(n) || 0;
    if (v === 0) return '';
    if (v >= 1_048_576) return (v / 1_048_576).toFixed(1) + 'MB';
    if (v >= 1_024) return (v / 1_024).toFixed(1) + 'KB';
    return v + 'B';
  }

  const statCards = $derived(stats ? [
    { label: 'Total', value: stats.total },
    { label: 'Allowed', value: stats.allowed, colorClass: 'text-allowed' },
    { label: 'Denied', value: stats.denied, colorClass: 'text-denied' },
    { label: 'Avg Latency', value: Math.round(stats.avg_latency) + 'ms' },
  ] : []);

  const allowedColor = colors.allowed;
  const deniedColor = colors.denied;
</script>

<div class="flex h-full overflow-hidden">
  <div class="flex-1 min-w-0 flex flex-col overflow-hidden">
    {#if stats}
      <StatCards cards={statCards} />
    {/if}

    <div class="grid grid-cols-3 gap-3 px-3 pb-2">
      <div class="bg-base-200/30 rounded-lg p-2">
        <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Requests Over Time</div>
        <div class="h-36 w-full">
          {#if timeData.length > 0}
            <BarChart
              data={timeData}
              x="bucket"
              series={[
                { key: 'allowed', label: 'Allowed', color: allowedColor },
                { key: 'denied', label: 'Denied', color: deniedColor },
              ]}
              seriesLayout="stack"
              props={{ legend: { placement: 'bottom' } }}
            />
          {/if}
        </div>
      </div>
      <div class="bg-base-200/30 rounded-lg p-2">
        <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">HTTP Methods</div>
        <div class="h-36 w-full">
          {#if methodSeries.length > 0}
            <PieChart
              key="key"
              value="value"
              innerRadius={40}
              series={methodSeries}
              props={{ legend: { placement: 'bottom' } }}
            />
          {/if}
        </div>
      </div>
      <div class="bg-base-200/30 rounded-lg p-2">
        <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Top Domains</div>
        <div class="h-36 w-full">
          {#if domainData.length > 0}
            <BarChart
              data={domainData}
              y="domain"
              series={[
                { key: 'allowed', label: 'Allowed', color: allowedColor },
                { key: 'denied', label: 'Denied', color: deniedColor },
              ]}
              seriesLayout="stack"
              orientation="horizontal"
              props={{ legend: { placement: 'bottom' } }}
            />
          {/if}
        </div>
      </div>
    </div>

    <div class="flex items-center gap-2 px-3 py-2 border-b border-base-200">
      <input type="text" class="input input-xs input-bordered flex-1 font-mono" placeholder="Search domain, path, method..." value={search} oninput={(e) => onSearch(e.currentTarget.value)} />
      <span class="text-xs text-base-content/40">{rows.length} events</span>
    </div>

    <div class="flex-1 overflow-auto">
      {#if !loaded}
        <div class="flex items-center justify-center h-32"><span class="loading loading-spinner loading-md"></span></div>
      {:else if rows.length === 0}
        <div class="flex items-center justify-center h-32 text-base-content/40 text-sm">No network events recorded.</div>
      {:else}
        <table class="table table-xs table-pin-rows">
          <thead><tr>
            <th class="w-20">Time</th><th>Domain</th><th>Method + Path</th><th class="w-14">Status</th><th class="w-20">Decision</th><th class="w-16">Duration</th><th class="w-16">Bytes</th>
          </tr></thead>
          <tbody>
            {#each rows as row}
              <tr class="hover:bg-base-200/40 cursor-pointer transition-colors" onclick={() => selectRow(row)}>
                <td class="font-mono text-base-content/40">{fmtTime(row.timestamp)}</td>
                <td class="font-mono truncate max-w-40">{row.domain}</td>
                <td class="font-mono truncate max-w-60 text-base-content/60">
                  {#if row.method}<span class="text-base-content/80">{row.method}</span>{/if}
                  {row.path ?? ''}
                </td>
                <td class="font-mono text-base-content/50">{row.status_code ?? ''}</td>
                <td><span class="badge badge-xs {row.decision === 'allowed' ? 'bg-allowed/15 text-allowed' : 'bg-denied/15 text-denied'}">{row.decision}</span></td>
                <td class="font-mono text-base-content/40">{row.duration_ms ? row.duration_ms + 'ms' : ''}</td>
                <td class="font-mono text-base-content/40">{fmtBytes((Number(row.bytes_sent) || 0) + (Number(row.bytes_received) || 0))}</td>
              </tr>
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
