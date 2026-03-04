<script lang="ts">
  import { onMount } from 'svelte';
  import { queryDb, queryAll, queryOne } from '../../db';
  import {
    TOOLS_STATS_SQL, TOOLS_TOP_TOOLS_SQL, TOOLS_TOP_SERVERS_SQL, TOOLS_OVER_TIME_SQL,
    TOOLS_UNIFIED_SQL, TOOLS_UNIFIED_SEARCH_SQL,
  } from '../../sql';
  import { colors, serverColor } from '../../css-var';
  import { BarChart } from 'layerchart';
  import type { DetailSelection } from '../../types';
  import DetailPanel from './DetailPanel.svelte';
  import StatCards from './StatCards.svelte';

  let stats = $state<{ total: number; native: number; mcp: number; allowed: number; denied: number } | null>(null);
  let rows = $state<Record<string, unknown>[]>([]);
  let detail = $state<DetailSelection | null>(null);
  let loaded = $state(false);
  let search = $state('');
  let searchDebounced = $state('');
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  let timeData = $state<{ bucket: string; native: number; mcp: number }[]>([]);
  let toolData = $state<{ tool_name: string; cnt: number; color: string }[]>([]);
  let serverData = $state<{ server_name: string; cnt: number; color: string }[]>([]);

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
    try { stats = queryOne(await queryDb(TOOLS_STATS_SQL)); }
    catch (e) { console.error('Tools stats failed:', e); }
  }

  async function loadCharts() {
    try {
      const [timeRes, toolsRes, serversRes] = await Promise.all([
        queryDb(TOOLS_OVER_TIME_SQL),
        queryDb(TOOLS_TOP_TOOLS_SQL),
        queryDb(TOOLS_TOP_SERVERS_SQL),
      ]);

      const nativeColor = colors.allowed;
      const mcpColor = colors.providerAnthropic;

      const timeRows = queryAll<{ bucket: number; native: number; mcp: number }>(timeRes);
      timeData = timeRows.map((r, i) => ({ bucket: String(i + 1), native: r.native, mcp: r.mcp }));

      const toolRows = queryAll<{ tool_name: string; cnt: number; source: string }>(toolsRes);
      toolData = toolRows.map(r => ({ tool_name: r.tool_name, cnt: r.cnt, color: r.source === 'mcp' ? mcpColor : nativeColor }));

      const srvRows = queryAll<{ server_name: string; cnt: number }>(serversRes);
      serverData = srvRows.map((r, i) => ({ server_name: r.server_name, cnt: r.cnt, color: serverColor('', i) }));
    } catch (e) { console.error('Tools charts failed:', e); }
  }

  async function loadEvents() {
    loaded = false;
    try {
      const q = searchDebounced.trim();
      if (q) {
        const like = `%${q}%`;
        rows = queryAll(await queryDb(TOOLS_UNIFIED_SEARCH_SQL, [like, like, like, like]));
      } else {
        rows = queryAll(await queryDb(TOOLS_UNIFIED_SQL));
      }
    } catch (e) { console.error('Tools tab load failed:', e); }
    loaded = true;
  }

  function selectRow(row: Record<string, unknown>) {
    detail = {
      type: 'tool',
      data: {
        tool_name: row.tool_name ?? row.method,
        arguments: row.arguments,
        origin: row.source === 'mcp' ? 'mcp' : 'native',
        content_preview: row.response_preview ?? undefined,
        is_error: row.error_message ? 1 : 0,
      },
    };
  }

  function fmtTime(ts: unknown): string {
    if (!ts) return '';
    const s = String(ts);
    const idx = s.indexOf('T');
    return idx >= 0 ? s.substring(idx + 1, idx + 9) : s;
  }

  function fmtDuration(ms: unknown): string {
    if (!ms) return '';
    const n = Number(ms);
    if (n >= 60_000) return (n / 60_000).toFixed(1) + 'm';
    if (n >= 1_000) return (n / 1_000).toFixed(1) + 's';
    return n + 'ms';
  }

  function fmtBytes(b: unknown): string {
    if (!b) return '';
    const n = Number(b);
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'MB';
    if (n >= 1_000) return (n / 1_000).toFixed(1) + 'KB';
    return n + 'B';
  }

  const statCards = $derived(stats ? [
    { label: 'Total', value: stats.total },
    { label: 'Native', value: stats.native, colorClass: 'text-allowed' },
    { label: 'MCP', value: stats.mcp, colorClass: 'text-provider-anthropic' },
    { label: 'Denied', value: stats.denied, colorClass: 'text-denied' },
  ] : []);

  const nativeColor = colors.allowed;
  const mcpColor = colors.providerAnthropic;
</script>

<div class="flex h-full overflow-hidden">
  <div class="flex-1 min-w-0 flex flex-col overflow-hidden">
    {#if stats}
      <StatCards cards={statCards} />
    {/if}

    <div class="grid grid-cols-3 gap-3 px-3 pb-2">
      <div class="bg-base-200/30 rounded-lg p-2">
        <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Calls Over Time</div>
        <div class="h-36 w-full">
          {#if timeData.length > 0}
            <BarChart
              data={timeData}
              x="bucket"
              series={[
                { key: 'native', label: 'Native', color: nativeColor },
                { key: 'mcp', label: 'MCP', color: mcpColor },
              ]}
              seriesLayout="stack"
              props={{ legend: { placement: 'bottom' } }}
            />
          {/if}
        </div>
      </div>
      <div class="bg-base-200/30 rounded-lg p-2">
        <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Top Tools</div>
        <div class="h-36 w-full">
          {#if toolData.length > 0}
            <BarChart
              data={toolData}
              y="tool_name"
              series={[{ key: 'cnt', label: 'Calls', color: nativeColor }]}
              orientation="horizontal"
              props={{ legend: { placement: 'bottom' } }}
            />
          {/if}
        </div>
      </div>
      <div class="bg-base-200/30 rounded-lg p-2">
        <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">Top Servers</div>
        <div class="h-36 w-full">
          {#if serverData.length > 0}
            <BarChart
              data={serverData}
              x="server_name"
              series={[{ key: 'cnt', label: 'Calls', color: mcpColor }]}
              props={{ legend: { placement: 'bottom' } }}
            />
          {/if}
        </div>
      </div>
    </div>

    <div class="flex items-center gap-2 px-3 py-2 border-b border-base-200">
      <input type="text" class="input input-xs input-bordered flex-1 font-mono" placeholder="Search tool, method, server, process..." value={search} oninput={(e) => onSearch(e.currentTarget.value)} />
      <span class="text-xs text-base-content/40">{rows.length} total</span>
    </div>

    <div class="flex-1 overflow-auto">
      {#if !loaded}
        <div class="flex items-center justify-center h-32"><span class="loading loading-spinner loading-md"></span></div>
      {:else if rows.length === 0}
        <div class="flex items-center justify-center h-32 text-base-content/40 text-sm">No tool calls recorded.</div>
      {:else}
        <table class="table table-xs table-pin-rows table-fixed w-full">
          <thead><tr>
            <th class="w-18">Time</th><th class="w-20">Process</th><th class="w-16">Server</th><th>Tool</th><th>Method</th><th class="w-18">Decision</th><th class="w-16 text-right">Duration</th><th class="w-14 text-right">Size</th>
          </tr></thead>
          <tbody>
            {#each rows as row}
              <tr class="hover:bg-base-200/40 cursor-pointer transition-colors" onclick={() => selectRow(row)}>
                <td class="font-mono text-base-content/40 truncate">{fmtTime(row.timestamp)}</td>
                <td class="text-base-content/50 truncate">{row.process_name ?? ''}</td>
                <td class="text-base-content/50 truncate">{row.server_name}</td>
                <td class="font-mono truncate">{row.tool_name ?? ''}</td>
                <td class="font-mono text-base-content/70 truncate">{row.method ?? ''}</td>
                <td>
                  {#if row.decision === 'allowed'}
                    <span class="badge badge-xs w-14 text-center bg-allowed/15 text-allowed border-0">allowed</span>
                  {:else}
                    <span class="badge badge-xs w-14 text-center bg-denied/15 text-denied border-0">denied</span>
                  {/if}
                </td>
                <td class="font-mono text-base-content/40 text-right">{fmtDuration(row.duration_ms)}</td>
                <td class="font-mono text-base-content/40 text-right">{fmtBytes(row.bytes)}</td>
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
