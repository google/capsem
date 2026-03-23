<script lang="ts">
  import { onMount } from 'svelte';
  import { listSnapshots } from '../../api';
  import StatCards from './StatCards.svelte';

  interface Snapshot {
    checkpoint: string;
    slot: number;
    origin: string;
    name: string | null;
    hash: string | null;
    age: string;
  }

  interface SnapshotData {
    snapshots: Snapshot[];
    auto_max: number;
    manual_max: number;
    manual_available: number;
  }

  let data = $state<SnapshotData | null>(null);
  let loaded = $state(false);
  let deleting = $state<string | null>(null);

  onMount(() => { load(); });

  async function load() {
    loaded = false;
    try {
      data = await listSnapshots();
    } catch (e) {
      console.error('Snapshots load failed:', e);
    }
    loaded = true;
  }

  async function handleDelete(checkpoint: string) {
    deleting = checkpoint;
    try {
      const { deleteSnapshot } = await import('../../api');
      await deleteSnapshot(checkpoint);
      await load();
    } catch (e) {
      console.error('Delete snapshot failed:', e);
    }
    deleting = null;
  }

  const autoSnaps = $derived(data?.snapshots.filter(s => s.origin === 'auto') ?? []);
  const manualSnaps = $derived(data?.snapshots.filter(s => s.origin === 'manual') ?? []);

  const statCards = $derived(data ? [
    { label: 'Total', value: data.snapshots.length },
    { label: 'Auto', value: autoSnaps.length, sub: `of ${data.auto_max}` },
    { label: 'Manual', value: manualSnaps.length, sub: `of ${data.manual_max}` },
    { label: 'Available', value: data.manual_available, sub: 'manual slots' },
  ] : []);

  function truncHash(hash: string | null): string {
    if (!hash) return '';
    return hash.substring(0, 12);
  }
</script>

<div class="flex-1 min-w-0 flex flex-col overflow-hidden">
  {#if data}
    <StatCards cards={statCards} />
  {/if}

  {#if !loaded}
    <div class="flex items-center justify-center h-32">
      <span class="loading loading-spinner loading-md"></span>
    </div>
  {:else if !data || data.snapshots.length === 0}
    <div class="flex items-center justify-center h-32 text-base-content/40 text-sm">
      No snapshots yet.
    </div>
  {:else}
    <div class="flex-1 overflow-auto">
      {#if manualSnaps.length > 0}
        <div class="px-3 pt-2">
          <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">
            Named Snapshots
          </div>
        </div>
        <table class="table table-xs table-pin-rows">
          <thead><tr>
            <th class="w-20">Slot</th>
            <th>Name</th>
            <th class="w-24">Age</th>
            <th class="w-32">Hash</th>
            <th class="w-16"></th>
          </tr></thead>
          <tbody>
            {#each manualSnaps as snap}
              <tr class="hover:bg-base-200/40 transition-colors">
                <td>
                  <span class="badge badge-xs bg-snap-manual/15 text-snap-manual">{snap.checkpoint}</span>
                </td>
                <td class="font-medium">{snap.name ?? ''}</td>
                <td class="text-base-content/50 text-xs">{snap.age}</td>
                <td>
                  {#if snap.hash}
                    <code class="text-[10px] text-base-content/30 font-mono tracking-tight">{truncHash(snap.hash)}</code>
                  {/if}
                </td>
                <td>
                  <button
                    class="btn btn-ghost btn-xs text-base-content/30 hover:text-denied"
                    disabled={deleting === snap.checkpoint}
                    onclick={() => handleDelete(snap.checkpoint)}
                  >
                    {#if deleting === snap.checkpoint}
                      <span class="loading loading-spinner loading-xs"></span>
                    {:else}
                      <svg xmlns="http://www.w3.org/2000/svg" class="w-3.5 h-3.5" viewBox="0 0 20 20" fill="currentColor">
                        <path fill-rule="evenodd" d="M8.75 1A2.75 2.75 0 006 3.75v.443c-.795.077-1.584.176-2.365.298a.75.75 0 10.23 1.482l.149-.022.841 10.518A2.75 2.75 0 007.596 19h4.807a2.75 2.75 0 002.742-2.53l.841-10.519.149.023a.75.75 0 00.23-1.482A41.03 41.03 0 0014 4.193V3.75A2.75 2.75 0 0011.25 1h-2.5zM10 4c.84 0 1.673.025 2.5.075V3.75c0-.69-.56-1.25-1.25-1.25h-2.5c-.69 0-1.25.56-1.25 1.25v.325C8.327 4.025 9.16 4 10 4zM8.58 7.72a.75.75 0 00-1.5.06l.3 7.5a.75.75 0 101.5-.06l-.3-7.5zm4.34.06a.75.75 0 10-1.5-.06l-.3 7.5a.75.75 0 101.5.06l.3-7.5z" clip-rule="evenodd"/>
                      </svg>
                    {/if}
                  </button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}

      <div class="px-3 pt-3">
        <div class="text-[10px] font-semibold text-base-content/40 uppercase tracking-wider mb-1">
          Auto Snapshots
        </div>
      </div>
      <table class="table table-xs table-pin-rows">
        <thead><tr>
          <th class="w-20">Slot</th>
          <th>Origin</th>
          <th class="w-24">Age</th>
        </tr></thead>
        <tbody>
          {#each autoSnaps as snap}
            <tr class="hover:bg-base-200/40 transition-colors">
              <td>
                <span class="badge badge-xs badge-outline text-snap-auto">{snap.checkpoint}</span>
              </td>
              <td class="text-base-content/40 text-xs">auto</td>
              <td class="text-base-content/50 text-xs">{snap.age}</td>
            </tr>
          {/each}
          {#if autoSnaps.length === 0}
            <tr><td colspan="3" class="text-center text-base-content/30 text-xs py-4">No auto snapshots yet</td></tr>
          {/if}
        </tbody>
      </table>
    </div>
  {/if}
</div>
