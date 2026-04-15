<script lang="ts">
  import { onMount } from 'svelte';
  import { vmStore } from '../../stores/vms.svelte.ts';
  import { tabStore } from '../../stores/tabs.svelte.ts';
  import * as api from '../../api';
  import type { VmSummary } from '../../types/gateway';
  import type { GlobalStats } from '../../types/gateway';
  import { formatUptime, formatTokens, formatCost } from '../../format';
  import Modal from './Modal.svelte';
  import ArrowClockwise from 'phosphor-svelte/lib/ArrowClockwise';
  import Pause from 'phosphor-svelte/lib/Pause';
  import Trash from 'phosphor-svelte/lib/Trash';
  import Play from 'phosphor-svelte/lib/Play';
  import Lightning from 'phosphor-svelte/lib/Lightning';
  import CircleNotch from 'phosphor-svelte/lib/CircleNotch';

  type SortKey = 'name' | 'status' | 'type';
  type SortDir = 'asc' | 'desc';

  let globalStats = $state<GlobalStats | null>(null);
  let statsLoading = $state(true);

  // True until first poll comes back
  let initialLoading = $derived(vmStore.serviceStatus === 'unknown');

  onMount(async () => {
    try {
      const stats = await api.getStats();
      globalStats = stats.global;
    } catch {
      // Offline -- globalStats stays null, cards show zeros
    } finally {
      statsLoading = false;
    }
  });

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

  // --- Modal state ---
  type DashModalKind = 'stop' | 'destroy' | null;
  let dashModalKind = $state<DashModalKind>(null);
  let dashModalVm = $state<VmSummary | null>(null);

  function openDashModal(e: MouseEvent, kind: DashModalKind, vm: VmSummary) {
    e.stopPropagation();
    dashModalVm = vm;
    dashModalKind = kind;
  }

  function closeDashModal() {
    dashModalKind = null;
    dashModalVm = null;
  }

  async function handleDashModalConfirm() {
    if (!dashModalVm) return;
    const id = dashModalVm.id;
    const kind = dashModalKind;
    closeDashModal();
    if (kind === 'stop') await vmStore.stop(id);
    else if (kind === 'destroy') await vmStore.delete(id);
  }

  async function handleResume(e: MouseEvent, vm: VmSummary) {
    e.stopPropagation();
    if (vm.name) await vmStore.resume(vm.name);
  }

  let creatingTemp = $state(false);

  async function createTemporary() {
    if (creatingTemp) return;
    creatingTemp = true;
    try {
      const { id, name } = await vmStore.provision({ ram_mb: 2048, cpus: 2, persistent: false });
      tabStore.openVM(id, name);
    } finally {
      creatingTemp = false;
    }
  }
</script>

<div class="p-6 max-w-5xl mx-auto">
  <!-- Sessions header -->
  <div class="flex items-center justify-between mb-6">
    <h2 class="text-2xl font-bold text-foreground">Sessions</h2>
    <button
      type="button"
      class="inline-flex items-center gap-x-2 bg-primary text-primary-foreground hover:bg-primary-hover rounded-lg px-4 py-2 text-sm font-medium transition-colors disabled:opacity-50 disabled:pointer-events-none"
      onclick={createTemporary}
      disabled={creatingTemp}
    >
      <Lightning size={16} weight="fill" />
      {creatingTemp ? 'Creating...' : 'New Session'}
    </button>
  </div>

  <!-- Sessions list -->
  {#if initialLoading}
    <div class="bg-card border border-card-line rounded-xl p-12 flex items-center justify-center gap-x-3">
      <CircleNotch size={18} class="text-muted-foreground-1 animate-spin" />
      <p class="text-muted-foreground-1 text-sm">Loading sessions...</p>
    </div>
  {:else if sorted.length === 0}
    <div class="bg-card border border-card-line rounded-xl p-12 flex items-center justify-center">
      <p class="text-muted-foreground-1 text-sm">No active sessions</p>
    </div>
  {:else}
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
                { key: 'uptime', label: 'Uptime' },
                { key: 'tokens', label: 'Tokens' },
                { key: 'cost', label: 'Cost' },
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
                      <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-surface" onclick={async (e: MouseEvent) => { e.stopPropagation(); await vmStore.restart(vm.id); }} aria-label="Restart" title="Restart">
                        <ArrowClockwise size={16} />
                      </button>
                      <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-surface" onclick={(e: MouseEvent) => openDashModal(e, 'stop', vm)} aria-label="Stop" title="Stop">
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
                    <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-destructive hover:bg-surface" onclick={(e: MouseEvent) => openDashModal(e, 'destroy', vm)} aria-label="Delete" title="Delete">
                      <Trash size={16} />
                    </button>
                  </div>
                </td>
                <td class="p-3 whitespace-nowrap text-sm font-medium text-foreground">{vm.name ?? vm.id}</td>
                <td class="p-3 whitespace-nowrap text-sm">
                  <span class="text-xs px-2 py-0.5 rounded-full {statusBadge(vm.status)}">{vm.status}</span>
                </td>
                <td class="p-3 whitespace-nowrap text-sm text-foreground">{vm.persistent ? 'persistent' : 'ephemeral'}</td>
                <td class="p-3 whitespace-nowrap text-sm text-muted-foreground-1 tabular-nums">{vm.uptime_secs != null ? formatUptime(vm.uptime_secs) : '--'}</td>
                <td class="p-3 whitespace-nowrap text-sm text-muted-foreground-1 tabular-nums">{vm.total_input_tokens != null ? formatTokens((vm.total_input_tokens ?? 0) + (vm.total_output_tokens ?? 0)) : '--'}</td>
                <td class="p-3 whitespace-nowrap text-sm text-muted-foreground-1 tabular-nums">{vm.total_estimated_cost != null ? formatCost(vm.total_estimated_cost) : '--'}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </div>
  {/if}

  <!-- Statistics -->
  <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mt-8 mb-3">Statistics</h3>
  {#if statsLoading}
    <div class="flex items-center gap-x-2 py-3">
      <CircleNotch size={14} class="text-muted-foreground-1 animate-spin" />
      <span class="text-xs text-muted-foreground-1">Loading statistics...</span>
    </div>
  {:else}
    <div class="grid grid-cols-4 gap-3">
      <div class="bg-card border border-card-line rounded-lg p-3">
        <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Sessions</div>
        <div class="text-lg font-semibold text-foreground">{globalStats?.total_sessions ?? 0}</div>
      </div>
      <div class="bg-card border border-card-line rounded-lg p-3">
        <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Total Tokens</div>
        <div class="text-lg font-semibold text-foreground">{formatTokens((globalStats?.total_input_tokens ?? 0) + (globalStats?.total_output_tokens ?? 0))}</div>
      </div>
      <div class="bg-card border border-card-line rounded-lg p-3">
        <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Total Cost</div>
        <div class="text-lg font-semibold text-foreground">{formatCost(globalStats?.total_estimated_cost ?? 0)}</div>
      </div>
      <div class="bg-card border border-card-line rounded-lg p-3">
        <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Requests</div>
        <div class="text-lg font-semibold text-foreground">{globalStats?.total_requests ?? 0}</div>
      </div>
    </div>
  {/if}
</div>

<Modal
  open={dashModalKind === 'stop'}
  title="Stop Session"
  confirmLabel="Stop"
  destructive
  onconfirm={handleDashModalConfirm}
  oncancel={closeDashModal}
>
  <p class="text-sm text-foreground">Stop <strong>{dashModalVm?.name ?? dashModalVm?.id}</strong>?</p>
  {#if dashModalVm && !dashModalVm.persistent}
    <p class="text-xs text-muted-foreground-1 mt-2">This is an ephemeral session. It will be destroyed.</p>
  {/if}
</Modal>

<Modal
  open={dashModalKind === 'destroy'}
  title="Destroy Session"
  confirmLabel="Destroy"
  destructive
  onconfirm={handleDashModalConfirm}
  oncancel={closeDashModal}
>
  <p class="text-sm text-foreground">Destroy <strong>{dashModalVm?.name ?? dashModalVm?.id}</strong>? This cannot be undone.</p>
</Modal>
