<script lang="ts">
  import { onMount } from 'svelte';
  import { queryDb, queryAll } from '../../db';
  import { NET_EVENTS_ALL_SQL, NET_EVENTS_SEARCH_SQL } from '../../sql';
  import type { DetailSelection } from '../../types';
  import DetailPanel from './DetailPanel.svelte';

  let totalCount = $state(0);
  let rows = $state<Record<string, unknown>[]>([]);
  let search = $state('');
  let searchDebounced = $state('');
  let detail = $state<DetailSelection | null>(null);
  let loaded = $state(false);
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
        const dataRes = await queryDb(NET_EVENTS_SEARCH_SQL, [like, like, like]);
        rows = queryAll<Record<string, unknown>>(dataRes);
      } else {
        const dataRes = await queryDb(NET_EVENTS_ALL_SQL);
        rows = queryAll<Record<string, unknown>>(dataRes);
      }
      totalCount = rows.length;
    } catch (e) {
      console.error('Network tab load failed:', e);
    }
    loaded = true;
  }

  function selectRow(row: Record<string, unknown>) {
    detail = { type: 'net_event', data: row };
  }

  function fmtTime(ts: unknown): string {
    if (!ts) return '';
    const s = String(ts);
    const idx = s.indexOf('T');
    if (idx >= 0) return s.substring(idx + 1, idx + 9);
    return s;
  }

  function fmtBytes(n: unknown): string {
    const v = Number(n) || 0;
    if (v === 0) return '';
    if (v >= 1_048_576) return (v / 1_048_576).toFixed(1) + 'MB';
    if (v >= 1_024) return (v / 1_024).toFixed(1) + 'KB';
    return v + 'B';
  }
</script>

<div class="flex h-full overflow-hidden">
  <div class="flex-1 min-w-0 flex flex-col overflow-hidden">
    <!-- Search -->
    <div class="flex items-center gap-2 px-3 py-2 border-b border-base-200">
      <input
        type="text"
        class="input input-xs input-bordered flex-1 font-mono"
        placeholder="Search domain, path, method..."
        value={search}
        oninput={(e) => onSearch(e.currentTarget.value)}
      />
      <span class="text-xs text-base-content/40">{totalCount} events</span>
    </div>

    <!-- Table -->
    <div class="flex-1 overflow-auto">
      {#if !loaded}
        <div class="flex items-center justify-center h-32">
          <span class="loading loading-spinner loading-md"></span>
        </div>
      {:else if rows.length === 0}
        <div class="flex items-center justify-center h-32 text-base-content/40 text-sm">
          No network events recorded.
        </div>
      {:else}
        <table class="table table-xs table-pin-rows">
          <thead>
            <tr>
              <th class="w-20">Time</th>
              <th>Domain</th>
              <th>Method + Path</th>
              <th class="w-14">Status</th>
              <th class="w-20">Decision</th>
              <th class="w-16">Duration</th>
              <th class="w-16">Bytes</th>
            </tr>
          </thead>
          <tbody>
            {#each rows as row}
              <tr
                class="hover:bg-base-200/40 cursor-pointer transition-colors"
                onclick={() => selectRow(row)}
              >
                <td class="font-mono text-base-content/40">{fmtTime(row.timestamp)}</td>
                <td class="font-mono truncate max-w-40">{row.domain}</td>
                <td class="font-mono truncate max-w-60 text-base-content/60">
                  {#if row.method}
                    <span class="text-base-content/80">{row.method}</span>
                  {/if}
                  {row.path ?? ''}
                </td>
                <td class="font-mono text-base-content/50">{row.status_code ?? ''}</td>
                <td>
                  <span class="badge badge-xs {row.decision === 'allowed' ? 'bg-allowed/15 text-allowed' : 'bg-denied/15 text-denied'}">{row.decision}</span>
                </td>
                <td class="font-mono text-base-content/40">{row.duration_ms ? row.duration_ms + 'ms' : ''}</td>
                <td class="font-mono text-base-content/40">{fmtBytes((Number(row.bytes_sent) || 0) + (Number(row.bytes_received) || 0))}</td>
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
