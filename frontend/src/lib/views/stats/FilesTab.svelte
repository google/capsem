<script lang="ts">
  import { onMount } from 'svelte';
  import { queryDb, queryAll, queryOne } from '../../db';
  import {
    FILE_STATS_SQL, FILE_ACTIONS_SQL, FILE_EVENTS_OVER_TIME_SQL,
    FILE_EVENTS_ALL_SQL, FILE_EVENTS_SEARCH_SQL,
  } from '../../sql';
  import { colors } from '../../css-var';
  import { BarChart, PieChart } from 'layerchart';
  import type { DetailSelection } from '../../types';
  import DetailPanel from './DetailPanel.svelte';
  import StatCards from './StatCards.svelte';

  let stats = $state<{ total: number; created: number; modified: number; deleted: number } | null>(null);
  let rows = $state<Record<string, unknown>[]>([]);
  let search = $state('');
  let searchDebounced = $state('');
  let detail = $state<DetailSelection | null>(null);
  let loaded = $state(false);
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  let actionData = $state<{ key: string; value: number; color: string }[]>([]);
  let timeData = $state<{ bucket: string; created: number; modified: number; deleted: number }[]>([]);

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
    try { stats = queryOne(await queryDb(FILE_STATS_SQL)); }
    catch (e) { console.error('File stats failed:', e); }
  }

  function actionColor(action: string): string {
    switch (action) {
      case 'created': return colors.fileCreated;
      case 'modified': return colors.fileModified;
      case 'deleted': return colors.fileDeleted;
      default: return colors.providerFallback;
    }
  }

  async function loadCharts() {
    try {
      const [actionRes, timeRes] = await Promise.all([
        queryDb(FILE_ACTIONS_SQL),
        queryDb(FILE_EVENTS_OVER_TIME_SQL),
      ]);

      const actionRows = queryAll<{ action: string; cnt: number }>(actionRes);
      actionData = actionRows.map(r => ({
        key: r.action,
        value: r.cnt,
        color: actionColor(r.action),
      }));

      const timeRows = queryAll<{ bucket: number; action: string; cnt: number }>(timeRes);
      const buckets = [...new Set(timeRows.map(r => r.bucket))].sort((a, b) => a - b);
      timeData = buckets.map((b, i) => {
        const created = timeRows.find(r => r.bucket === b && r.action === 'created')?.cnt ?? 0;
        const modified = timeRows.find(r => r.bucket === b && r.action === 'modified')?.cnt ?? 0;
        const deleted = timeRows.find(r => r.bucket === b && r.action === 'deleted')?.cnt ?? 0;
        return { bucket: String(i + 1), created, modified, deleted };
      });
    } catch (e) { console.error('File charts failed:', e); }
  }

  async function loadEvents() {
    loaded = false;
    try {
      const q = searchDebounced.trim();
      if (q) {
        const like = `%${q}%`;
        rows = queryAll(await queryDb(FILE_EVENTS_SEARCH_SQL, [like]));
      } else {
        rows = queryAll(await queryDb(FILE_EVENTS_ALL_SQL));
      }
    } catch (e) { console.error('File events failed:', e); }
    loaded = true;
  }

  function selectRow(row: Record<string, unknown>) { detail = { type: 'file_event', data: row }; }

  function fmtTime(ts: unknown): string {
    if (!ts) return '';
    const s = String(ts);
    const idx = s.indexOf('T');
    return idx >= 0 ? s.substring(idx + 1, idx + 9) : s;
  }

  function fmtSize(n: unknown): string {
    const v = Number(n);
    if (!v && v !== 0) return '';
    if (v >= 1_048_576) return (v / 1_048_576).toFixed(1) + 'MB';
    if (v >= 1_024) return (v / 1_024).toFixed(1) + 'KB';
    return v + 'B';
  }

  const statCards = $derived(stats ? [
    { label: 'Total', value: stats.total },
    { label: 'Created', value: stats.created, colorClass: 'text-file-created' },
    { label: 'Modified', value: stats.modified, colorClass: 'text-file-modified' },
    { label: 'Deleted', value: stats.deleted, colorClass: 'text-file-deleted' },
  ] : []);

  const actionTotal = $derived(actionData.reduce((s, d) => s + d.value, 0));

  const createdColor = colors.fileCreated;
  const modifiedColor = colors.fileModified;
  const deletedColor = colors.fileDeleted;
</script>

<div class="flex h-full overflow-hidden">
  <div class="flex-1 min-w-0 flex flex-col overflow-hidden">
    {#if stats}
      <StatCards cards={statCards} />
    {/if}

    <div class="grid grid-cols-3 gap-3 px-3 pb-2">
      <div class="col-span-2 bg-base-200/30 rounded-lg p-2">
        <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Events Over Time</div>
        <div class="h-36 w-full">
          {#if timeData.length > 0}
            <BarChart
              data={timeData}
              x="bucket"
              series={[
                { key: 'created', label: 'Created', color: createdColor },
                { key: 'modified', label: 'Modified', color: modifiedColor },
                { key: 'deleted', label: 'Deleted', color: deletedColor },
              ]}
              seriesLayout="stack"
              props={{ legend: { placement: 'bottom' } }}
            />
          {/if}
        </div>
      </div>
      <div class="bg-base-200/30 rounded-lg p-2">
        <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Action Breakdown</div>
        {#if actionData.length > 0}
          <div class="h-36 w-full">
            <PieChart
              data={actionData}
              key="key"
              value="value"
              c="color"
              innerRadius={35}
              label={(d) => {
                const pct = actionTotal ? Math.round(d.value / actionTotal * 100) : 0;
                return `${d.key} ${pct}%`;
              }}
              legend
            />
          </div>
        {/if}
      </div>
    </div>

    <div class="flex items-center gap-2 px-3 py-2 border-b border-base-200">
      <input type="text" class="input input-xs input-bordered flex-1 font-mono" placeholder="Search file path..." value={search} oninput={(e) => onSearch(e.currentTarget.value)} />
      <span class="text-xs text-base-content/40">{rows.length} events</span>
    </div>

    <div class="flex-1 overflow-auto">
      {#if !loaded}
        <div class="flex items-center justify-center h-32"><span class="loading loading-spinner loading-md"></span></div>
      {:else if rows.length === 0}
        <div class="flex items-center justify-center h-32 text-base-content/40 text-sm">No file events recorded.</div>
      {:else}
        <table class="table table-xs table-pin-rows">
          <thead><tr>
            <th class="w-20">Time</th><th class="w-24">Action</th><th>Path</th><th class="w-20">Size</th>
          </tr></thead>
          <tbody>
            {#each rows as row}
              <tr class="hover:bg-base-200/40 cursor-pointer transition-colors" onclick={() => selectRow(row)}>
                <td class="font-mono text-base-content/40">{fmtTime(row.timestamp)}</td>
                <td><span class="badge badge-xs {row.action === 'deleted' ? 'bg-file-deleted/15 text-file-deleted' : row.action === 'created' ? 'bg-file-created/15 text-file-created' : 'bg-file-modified/15 text-file-modified'}">{row.action}</span></td>
                <td class="font-mono truncate max-w-lg">{row.path}</td>
                <td class="font-mono text-base-content/40">{row.size != null ? fmtSize(row.size) : ''}</td>
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
