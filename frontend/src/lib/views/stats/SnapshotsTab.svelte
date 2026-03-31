<script lang="ts">
  import { onMount } from 'svelte';
  import { queryDb, queryAll, queryOne } from '../../db';
  import { SNAPSHOT_STATS_SQL, SNAPSHOT_LIST_SQL } from '../../sql';
  import StatCards from './StatCards.svelte';

  interface SnapshotRow {
    id: number;
    timestamp: string;
    slot: number;
    origin: string;
    name: string | null;
    files_count: number;
    created: number;
    modified: number;
    deleted: number;
  }

  let stats = $state<{ total: number; auto_count: number; manual_count: number } | null>(null);
  let rows = $state<SnapshotRow[]>([]);
  let loaded = $state(false);

  onMount(() => { loadAll(); });

  async function loadAll() {
    await Promise.all([loadStats(), loadSnapshots()]);
  }

  async function loadStats() {
    try { stats = queryOne(await queryDb(SNAPSHOT_STATS_SQL)); }
    catch (e) { console.error('Snapshot stats failed:', e); }
  }

  async function loadSnapshots() {
    loaded = false;
    try { rows = queryAll(await queryDb(SNAPSHOT_LIST_SQL)); }
    catch (e) { console.error('Snapshot list failed:', e); }
    loaded = true;
  }

  function fmtAge(ts: string): string {
    if (!ts) return '';
    const d = new Date(ts);
    const now = Date.now();
    const mins = Math.floor((now - d.getTime()) / 60000);
    if (mins <= 0) return 'just now';
    if (mins === 1) return '1 min ago';
    if (mins < 60) return `${mins} min ago`;
    const hrs = Math.floor(mins / 60);
    return `${hrs} hr ago`;
  }

  const statCards = $derived(stats ? [
    { label: 'Total', value: stats.total },
    { label: 'Auto', value: stats.auto_count },
    { label: 'Manual', value: stats.manual_count },
  ] : []);
</script>

<div class="flex-1 min-w-0 flex flex-col overflow-hidden">
  {#if stats}
    <StatCards cards={statCards} />
  {/if}

  {#if !loaded}
    <div class="flex items-center justify-center h-32">
      <span class="loading loading-spinner loading-md"></span>
    </div>
  {:else if rows.length === 0}
    <div class="flex items-center justify-center h-32 text-base-content/40 text-sm">
      No snapshots yet.
    </div>
  {:else}
    <div class="flex-1 overflow-auto">
      <table class="table table-xs table-pin-rows">
        <thead><tr>
          <th class="w-20">Slot</th>
          <th>Name</th>
          <th class="w-20">Age</th>
          <th class="w-16 text-right">Files</th>
          <th class="w-16 text-right">Changes</th>
          <th class="w-16 text-right">Added</th>
          <th class="w-16 text-right">Modified</th>
          <th class="w-16 text-right">Deleted</th>
        </tr></thead>
        <tbody>
          {#each rows as snap}
            {@const total = snap.created + snap.modified + snap.deleted}
            <tr class="hover:bg-base-200/40 transition-colors">
              <td>
                {#if snap.origin === 'manual'}
                  <span class="badge badge-xs bg-snap-manual/15 text-snap-manual">cp-{snap.slot}</span>
                {:else}
                  <span class="badge badge-xs badge-outline text-snap-auto">cp-{snap.slot}</span>
                {/if}
              </td>
              <td class="font-medium">{snap.name ?? ''}</td>
              <td class="text-base-content/50 text-xs">{fmtAge(snap.timestamp)}</td>
              <td class="text-right tabular-nums text-base-content/40">{snap.files_count || ''}</td>
              <td class="text-right tabular-nums">{total || ''}</td>
              <td class="text-right tabular-nums">
                {#if snap.created > 0}
                  <span class="text-info">{snap.created}</span>
                {/if}
              </td>
              <td class="text-right tabular-nums">
                {#if snap.modified > 0}
                  <span class="text-warning">{snap.modified}</span>
                {/if}
              </td>
              <td class="text-right tabular-nums">
                {#if snap.deleted > 0}
                  <span class="text-secondary">{snap.deleted}</span>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>
