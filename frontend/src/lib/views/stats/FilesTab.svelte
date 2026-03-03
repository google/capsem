<script lang="ts">
  import { onMount } from 'svelte';
  import { queryDb, queryAll } from '../../db';
  import { FILE_EVENTS_ALL_SQL, FILE_EVENTS_SEARCH_SQL } from '../../sql';
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
        const dataRes = await queryDb(FILE_EVENTS_SEARCH_SQL, [like]);
        rows = queryAll<Record<string, unknown>>(dataRes);
      } else {
        const dataRes = await queryDb(FILE_EVENTS_ALL_SQL);
        rows = queryAll<Record<string, unknown>>(dataRes);
      }
      totalCount = rows.length;
    } catch (e) {
      console.error('Files tab load failed:', e);
    }
    loaded = true;
  }

  function selectRow(row: Record<string, unknown>) {
    detail = { type: 'file_event', data: row };
  }

  function fmtTime(ts: unknown): string {
    if (!ts) return '';
    const s = String(ts);
    const idx = s.indexOf('T');
    if (idx >= 0) return s.substring(idx + 1, idx + 9);
    return s;
  }

  function fmtSize(n: unknown): string {
    const v = Number(n);
    if (!v && v !== 0) return '';
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
        placeholder="Search file path..."
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
          No file events recorded.
        </div>
      {:else}
        <table class="table table-xs table-pin-rows">
          <thead>
            <tr>
              <th class="w-20">Time</th>
              <th class="w-24">Action</th>
              <th>Path</th>
              <th class="w-20">Size</th>
            </tr>
          </thead>
          <tbody>
            {#each rows as row}
              <tr
                class="hover:bg-base-200/40 cursor-pointer transition-colors"
                onclick={() => selectRow(row)}
              >
                <td class="font-mono text-base-content/40">{fmtTime(row.timestamp)}</td>
                <td>
                  <span class="badge badge-xs {row.action === 'deleted' ? 'bg-file-deleted/15 text-file-deleted' : row.action === 'created' ? 'bg-file-created/15 text-file-created' : 'bg-file-modified/15 text-file-modified'}">{row.action}</span>
                </td>
                <td class="font-mono truncate max-w-lg">{row.path}</td>
                <td class="font-mono text-base-content/40">{row.size != null ? fmtSize(row.size) : ''}</td>
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
