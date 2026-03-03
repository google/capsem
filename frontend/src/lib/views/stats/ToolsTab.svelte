<script lang="ts">
  import { onMount } from 'svelte';
  import { queryDb, queryAll } from '../../db';
  import { TOOLS_UNIFIED_SQL, TOOLS_UNIFIED_SEARCH_SQL } from '../../sql';
  import type { DetailSelection } from '../../types';
  import DetailPanel from './DetailPanel.svelte';

  let rows = $state<Record<string, unknown>[]>([]);
  let detail = $state<DetailSelection | null>(null);
  let loaded = $state(false);
  let search = $state('');
  let searchDebounced = $state('');
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;

  onMount(() => load());

  function onSearch(value: string) {
    search = value;
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      searchDebounced = value;
      load();
    }, 300);
  }

  async function load() {
    loaded = false;
    try {
      const q = searchDebounced.trim();
      if (q) {
        const like = `%${q}%`;
        const res = await queryDb(TOOLS_UNIFIED_SEARCH_SQL, [like, like, like, like]);
        rows = queryAll<Record<string, unknown>>(res);
      } else {
        const res = await queryDb(TOOLS_UNIFIED_SQL);
        rows = queryAll<Record<string, unknown>>(res);
      }
    } catch (e) {
      console.error('Tools tab load failed:', e);
    }
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

  function truncate(text: unknown, len: number): string {
    if (!text) return '';
    const s = String(text);
    if (s.length <= len) return s;
    return s.substring(0, len) + '...';
  }

  function fmtTime(ts: unknown): string {
    if (!ts) return '';
    const s = String(ts);
    const idx = s.indexOf('T');
    if (idx >= 0) return s.substring(idx + 1, idx + 9);
    return s;
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
</script>

<div class="flex h-full overflow-hidden">
  <div class="flex-1 min-w-0 flex flex-col overflow-hidden">
    <!-- Header with search -->
    <div class="flex items-center gap-2 px-3 py-2 border-b border-base-200">
      <input
        type="text"
        class="input input-xs input-bordered flex-1 font-mono"
        placeholder="Search tool, method, server, process..."
        value={search}
        oninput={(e) => onSearch(e.currentTarget.value)}
      />
      <span class="text-xs text-base-content/40">{rows.length} total</span>
    </div>

    <div class="flex-1 overflow-auto">
      {#if !loaded}
        <div class="flex items-center justify-center h-32">
          <span class="loading loading-spinner loading-md"></span>
        </div>
      {:else if rows.length === 0}
        <div class="flex items-center justify-center h-32 text-base-content/40 text-sm">
          No tool calls recorded.
        </div>
      {:else}
        <table class="table table-xs table-pin-rows table-fixed w-full">
          <thead>
            <tr>
              <th class="w-18">Time</th>
              <th class="w-20">Process</th>
              <th class="w-16">Server</th>
              <th>Tool</th>
              <th>Method</th>
              <th class="w-18">Decision</th>
              <th class="w-16 text-right">Duration</th>
              <th class="w-14 text-right">Size</th>
            </tr>
          </thead>
          <tbody>
            {#each rows as row}
              <tr
                class="hover:bg-base-200/40 cursor-pointer transition-colors"
                onclick={() => selectRow(row)}
              >
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

  <!-- Right detail panel -->
  {#if detail}
    <DetailPanel selection={detail} onClose={() => { detail = null; }} />
  {/if}
</div>
