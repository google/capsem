<script lang="ts">
  import { vmStore } from '../../stores/vms.svelte.ts';
  import { tabStore } from '../../stores/tabs.svelte.ts';
  import type { VmSummary } from '../../types/gateway';
  import ArrowClockwise from 'phosphor-svelte/lib/ArrowClockwise';
  import Pause from 'phosphor-svelte/lib/Pause';
  import Trash from 'phosphor-svelte/lib/Trash';
  import Play from 'phosphor-svelte/lib/Play';

  type SortKey = 'name' | 'status' | 'type';
  type SortDir = 'asc' | 'desc';

  let sortKey = $state<SortKey>('name');
  let sortDir = $state<SortDir>('asc');

  function toggleSort(key: SortKey) {
    if (sortKey === key) {
      sortDir = sortDir === 'asc' ? 'desc' : 'asc';
    } else {
      sortKey = key;
      sortDir = 'asc';
    }
  }

  let sorted = $derived.by(() => {
    const list = [...vmStore.vms];
    list.sort((a: VmSummary, b: VmSummary) => {
      let cmp = 0;
      switch (sortKey) {
        case 'name': cmp = (a.name ?? a.id).localeCompare(b.name ?? b.id); break;
        case 'status': cmp = a.status.localeCompare(b.status); break;
        case 'type': cmp = Number(a.persistent) - Number(b.persistent); break;
      }
      return sortDir === 'asc' ? cmp : -cmp;
    });
    return list;
  });

  // Map gateway status strings to badge styles
  const statusColor: Record<string, string> = {
    Running: 'bg-primary text-primary-foreground',
    Booting: 'bg-primary/60 text-primary-foreground',
    Stopped: 'bg-muted text-muted-foreground-1',
    Suspended: 'bg-warning text-warning-foreground',
    Error: 'bg-destructive text-destructive-foreground',
  };

  function statusBadge(status: string): string {
    return statusColor[status] ?? 'bg-muted text-muted-foreground-1';
  }

  async function handleStop(e: MouseEvent, vm: VmSummary) {
    e.stopPropagation();
    await vmStore.stop(vm.id);
  }

  async function handleDelete(e: MouseEvent, vm: VmSummary) {
    e.stopPropagation();
    await vmStore.delete(vm.id);
  }

  async function handleResume(e: MouseEvent, vm: VmSummary) {
    e.stopPropagation();
    if (vm.name) await vmStore.resume(vm.name);
  }
</script>

<div class="p-6 max-w-5xl mx-auto">
  <h2 class="text-2xl font-bold text-foreground mb-4">Sandboxes</h2>

  <div class="flex flex-col">
    <div class="overflow-x-auto [&::-webkit-scrollbar]:h-2 [&::-webkit-scrollbar-thumb]:rounded-none [&::-webkit-scrollbar-track]:bg-scrollbar-track [&::-webkit-scrollbar-thumb]:bg-scrollbar-thumb">
      <table class="min-w-full">
        <thead class="border-b border-table-line">
          <tr>
            <th scope="col" class="py-2 px-3 text-start font-normal text-sm text-muted-foreground-1">Actions</th>
            {#each [
              { key: 'name', label: 'Name' },
              { key: 'status', label: 'Status' },
              { key: 'type', label: 'Type' },
            ] as col (col.key)}
              <th scope="col" class="py-1 group text-start font-normal focus:outline-hidden">
                <button
                  type="button"
                  class="py-1 px-2.5 inline-flex items-center border border-transparent text-sm text-muted-foreground-1 rounded-md hover:border-line-2"
                  onclick={() => toggleSort(col.key as SortKey)}
                >
                  {col.label}
                  <svg class="size-3.5 ms-1 -me-0.5 text-muted-foreground" xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path class="{sortKey === col.key && sortDir === 'desc' ? 'text-primary' : ''}" d="m7 15 5 5 5-5"></path>
                    <path class="{sortKey === col.key && sortDir === 'asc' ? 'text-primary' : ''}" d="m7 9 5-5 5 5"></path>
                  </svg>
                </button>
              </th>
            {/each}
          </tr>
        </thead>

        <tbody class="divide-y divide-table-line">
          {#each sorted as vm (vm.id)}
            <tr class="hover:bg-muted-hover cursor-pointer" onclick={() => tabStore.openVM(vm.id, vm.name ?? vm.id)}>
              <td class="p-3 whitespace-nowrap">
                <div class="flex items-center gap-x-1">
                  {#if vm.status === 'Running'}
                    <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-surface" onclick={(e: MouseEvent) => handleStop(e, vm)} aria-label="Restart" title="Restart">
                      <ArrowClockwise size={16} />
                    </button>
                    <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-surface" onclick={(e: MouseEvent) => handleStop(e, vm)} aria-label="Stop" title="Stop">
                      <Pause size={16} />
                    </button>
                  {:else if vm.status === 'Stopped' || vm.status === 'Error'}
                    <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-primary hover:bg-surface" onclick={(e: MouseEvent) => handleResume(e, vm)} aria-label="Start" title="Start">
                      <Play size={16} />
                    </button>
                  {:else}
                    <div class="size-7"></div>
                    <div class="size-7"></div>
                  {/if}
                  <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-destructive hover:bg-surface" onclick={(e: MouseEvent) => handleDelete(e, vm)} aria-label="Delete" title="Delete">
                    <Trash size={16} />
                  </button>
                </div>
              </td>
              <td class="p-3 whitespace-nowrap text-sm font-medium text-foreground">{vm.name ?? vm.id}</td>
              <td class="p-3 whitespace-nowrap text-sm">
                <span class="text-xs px-2 py-0.5 rounded-full {statusBadge(vm.status)}">{vm.status}</span>
              </td>
              <td class="p-3 whitespace-nowrap text-sm text-foreground">{vm.persistent ? 'persistent' : 'ephemeral'}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
</div>
