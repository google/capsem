<script lang="ts">
  import { VmAction, type AssetStatus } from "@capsem/sdk";
  import { onMount } from 'svelte';
  import { vmStore } from '../../stores/vms.svelte.ts';
  import { tabStore } from '../../stores/tabs.svelte.ts';
  import * as api from '../../api';
  import type { VmSummary } from '../../types/gateway';
  import type { GlobalStats } from '../../types/gateway';
  import { formatUptime, formatTokens, formatCost } from '../../format';
  import { canOpenSession, hasVmAction, startAction, startLabel } from '../../vm-actions';
  import { sessionActionError } from '../../models/session-action-error';
  import { compareSessions, sessionTokens, type SessionSortKey, type SessionSortDirection } from '../../models/session-sort';
  import { watchVisibleRefresh } from '../../models/visible-refresh';
  import Modal from './Modal.svelte';
  import SessionCredentials from './SessionCredentials.svelte';
  import ImageCatalog from './ImageCatalog.svelte';
  import Pause from 'phosphor-svelte/lib/Pause';
  import Trash from 'phosphor-svelte/lib/Trash';
  import Play from 'phosphor-svelte/lib/Play';
  import CircleNotch from 'phosphor-svelte/lib/CircleNotch';
  import Warning from 'phosphor-svelte/lib/Warning';
  import X from 'phosphor-svelte/lib/X';
  import GitFork from 'phosphor-svelte/lib/GitFork';
  import Stop from 'phosphor-svelte/lib/Stop';

  const sessionColumns = [
    { key: 'name', label: 'Name' },
    { key: 'status', label: 'Status' },
    { key: 'uptime', label: 'Uptime' },
    { key: 'tokens', label: 'Tokens' },
    { key: 'cost', label: 'Cost' },
  ] satisfies { key: SessionSortKey; label: string }[];

  let globalStats = $state<GlobalStats | null>(null);
  let credentialSession = $state<VmSummary | null>(null);
  let statsLoading = $state(true);

  let initialLoading = $derived(!vmStore.polled);

  let assets = $state<AssetStatus | null>(null);
  let ready = $derived(assets?.ready === true);

  onMount(() => {
    void loadAssets();
    void loadStats();
    const stopRefreshing = watchVisibleRefresh(async () => {
      await Promise.all([loadAssets(), loadStats()]);
    }, 30_000);
    const progressPoll = window.setInterval(() => {
      if (!document.hidden && assets?.downloading) void loadAssets();
    }, 1000);
    return () => { stopRefreshing(); window.clearInterval(progressPoll); };
  });

  let sortKey = $state<SessionSortKey>('name');
  let sortDir = $state<SessionSortDirection>('asc');

  function toggleSort(key: SessionSortKey) {
    if (sortKey === key) {
      sortDir = sortDir === 'asc' ? 'desc' : 'asc';
    } else {
      sortKey = key;
      sortDir = 'asc';
    }
  }

  let allVms = $derived([...vmStore.vms].sort((a, b) => compareSessions(a, b, sortKey, sortDir)));
  let healthySessions = $derived(allVms.filter(vm => !isBrokenSession(vm)));
  let brokenSessions = $derived(allVms.filter(isBrokenSession));

  const statusColor: Record<string, string> = {
    Running: 'bg-primary text-primary-foreground',
    Booting: 'bg-primary/60 text-primary-foreground',
    Stopped: 'bg-muted text-muted-foreground-1',
    Suspended: 'bg-warning text-warning-foreground',
    Incompatible: 'bg-destructive text-destructive-foreground',
    Error: 'bg-destructive text-destructive-foreground',
  };

  function statusBadge(status: string): string {
    return statusColor[status] ?? 'bg-muted text-muted-foreground-1';
  }

  function isBrokenSession(vm: VmSummary): boolean {
    return vm.status === 'Defunct' || vm.status === 'Incompatible';
  }

  // --- Modal state ---
  type DashModalKind = 'stop' | 'delete' | null;
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
    await runAction(async () => {
      if (kind === 'stop') {
        await vmStore.stop(id);
      } else if (kind === 'delete') {
        const tab = tabStore.tabs.find(t => t.vmId === id);
        await vmStore.delete(id);
        if (tab) tabStore.close(tab.id);
      }
    });
  }

  async function handleStart(e: MouseEvent, vm: VmSummary) {
    e.stopPropagation();
    await runAction(() => vmStore.resume(vm.id));
  }

  async function handlePause(e: MouseEvent, vm: VmSummary) {
    e.stopPropagation();
    await runAction(() => vmStore.suspend(vm.id));
  }

  async function handleFork(e: MouseEvent, vm: VmSummary) {
    e.stopPropagation();
    const baseName = vm.name ?? vm.id;
    const name = prompt('Fork name:', `${baseName}-fork`);
    if (name?.trim()) {
      await runAction(() => vmStore.fork(vm.id, { name: name.trim() }));
    }
  }

  async function runAction(action: () => Promise<unknown>) {
    try {
      await action();
      actionError = null;
    } catch (error) {
      actionError = sessionActionError(error);
    }
  }

  let purging = $state(false);
  let actionError = $state<string | null>(null);

  let assetsRefreshInFlight = false;

  async function loadAssets() {
    if (assetsRefreshInFlight) return;
    assetsRefreshInFlight = true;
    const wasDownloading = assets?.downloading === true;
    try {
      assets = await api.getAssetsStatus();
      if (wasDownloading && !assets.downloading) await vmStore.refresh();
    } catch {
      assets = null;
    } finally {
      assetsRefreshInFlight = false;
    }
  }

  async function loadStats() {
    try {
      globalStats = (await api.getStats()).global;
    } catch {
      globalStats = null;
    } finally {
      statsLoading = false;
    }
  }

  function parseApiError(e: unknown): string {
    if (!(e instanceof Error)) return 'An unexpected error occurred';
    const msg = e.message;
    // ApiError format: "API error 500: {"error":"..."}"
    const jsonMatch = msg.match(/\{[^]*\}/);
    if (jsonMatch) {
      try {
        const parsed = JSON.parse(jsonMatch[0]);
        if (parsed.error) return parsed.error;
      } catch { /* fall through */ }
    }
    // Strip "API error NNN: " prefix
    const stripped = msg.replace(/^API error \d+:\s*/, '');
    return stripped || msg;
  }

  async function prepareRuntime() {
    if (ready) return;
    assets = await api.ensureAssets();
    const deadline = Date.now() + 900_000;
    while (assets.downloading && Date.now() < deadline) {
      await new Promise(resolve => window.setTimeout(resolve, 1000));
      assets = await api.getAssetsStatus();
    }
    if (!assets.ready) throw new Error(assets.reconcile_error ?? 'Capsem setup has not finished. Try again.');
  }

  async function handlePurgeBroken() {
    if (purging) return;
    actionError = null;
    purging = true;
    try {
      const ids = await vmStore.deleteBroken();
      for (const tab of [...tabStore.tabs]) {
        if (tab.vmId && ids.includes(tab.vmId)) tabStore.close(tab.id);
      }
    } catch (err) {
      actionError = parseApiError(err);
    } finally {
      purging = false;
    }
  }
</script>

{#snippet sessionTable(vms: VmSummary[])}
  <div class="flex flex-col">
    <div class="max-h-[50vh] overflow-y-auto overflow-x-auto [&::-webkit-scrollbar]:h-2 [&::-webkit-scrollbar]:w-2 [&::-webkit-scrollbar-thumb]:rounded-none [&::-webkit-scrollbar-track]:bg-scrollbar-track [&::-webkit-scrollbar-thumb]:bg-scrollbar-thumb">
      <table class="min-w-full">
        <thead class="border-b border-table-line">
          <tr>
            {#each sessionColumns as col (col.key)}
              <th scope="col" class="py-1 group text-start font-normal focus:outline-hidden" aria-sort={sortKey === col.key ? (sortDir === 'asc' ? 'ascending' : 'descending') : 'none'}>
                <button
                  type="button"
                  class="py-1 px-2.5 inline-flex items-center border border-transparent text-sm text-muted-foreground-1 rounded-md hover:border-line-2"
                  onclick={() => toggleSort(col.key)}
                >
                  {col.label}
                  <svg class="size-3.5 ms-1 -me-0.5 text-muted-foreground" xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path class="{sortKey === col.key && sortDir === 'desc' ? 'text-primary' : ''}" d="m7 15 5 5 5-5"></path>
                    <path class="{sortKey === col.key && sortDir === 'asc' ? 'text-primary' : ''}" d="m7 9 5-5 5 5"></path>
                  </svg>
                </button>
              </th>
            {/each}
            <th scope="col" class="py-2 px-3 text-end font-normal text-sm text-muted-foreground-1">Actions</th>
          </tr>
        </thead>

        <tbody class="divide-y divide-table-line">
          {#each vms as vm (vm.id)}
            {@const tokens = sessionTokens(vm)}
            <tr
              class="{canOpenSession(vm) ? 'hover:bg-muted-hover cursor-pointer' : 'opacity-60 cursor-default'}"
              onclick={() => { if (canOpenSession(vm)) tabStore.openVM(vm.id, vm.name ?? vm.id); }}
            >
              <td class="p-3 whitespace-nowrap text-sm font-medium text-foreground">{vm.name ?? vm.id}</td>
              <td class="p-3 whitespace-nowrap text-sm">
                <span class="text-xs px-2 py-0.5 rounded-full {statusBadge(vm.status)}">{vm.status}</span>
              </td>
              <td class="p-3 whitespace-nowrap text-sm text-muted-foreground-1 tabular-nums">{vm.uptime_secs != null ? formatUptime(vm.uptime_secs) : '--'}</td>
              <td class="p-3 whitespace-nowrap text-sm text-muted-foreground-1 tabular-nums">{tokens != null ? formatTokens(tokens) : '--'}</td>
              <td class="p-3 whitespace-nowrap text-sm text-muted-foreground-1 tabular-nums">{vm.total_estimated_cost != null ? formatCost(vm.total_estimated_cost) : '--'}</td>
              <td class="p-3 whitespace-nowrap text-end">
                <div class="inline-flex items-center gap-x-1">
                  {#if vm.status === 'Running' || vm.persistent}
                    <button type="button" class="py-1 px-2 text-xs rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-surface" onclick={(e: MouseEvent) => { e.stopPropagation(); credentialSession = vm; }}>Credentials</button>
                  {/if}
                  {#if hasVmAction(vm, VmAction.PAUSE)}
                    <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-surface" onclick={(e: MouseEvent) => handlePause(e, vm)} aria-label="Pause" title="Pause">
                      <Pause size={16} />
                    </button>
                  {/if}
                  {#if hasVmAction(vm, VmAction.STOP)}
                    <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-surface" onclick={(e: MouseEvent) => openDashModal(e, 'stop', vm)} aria-label="Stop" title="Stop">
                      <Stop size={16} />
                    </button>
                  {/if}
                  {#if hasVmAction(vm, startAction(vm))}
                    <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-primary hover:bg-surface" onclick={(e: MouseEvent) => handleStart(e, vm)} aria-label={startLabel(vm)} title={startLabel(vm)}>
                      <Play size={16} />
                    </button>
                  {/if}
                  {#if hasVmAction(vm, VmAction.FORK)}
                    <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-surface" onclick={(e: MouseEvent) => handleFork(e, vm)} aria-label="Fork" title="Fork">
                      <GitFork size={16} />
                    </button>
                  {/if}
                  {#if hasVmAction(vm, VmAction.DELETE)}
                    <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-destructive hover:bg-surface" onclick={(e: MouseEvent) => openDashModal(e, 'delete', vm)} aria-label="Delete" title="Delete">
                      <Trash size={16} />
                    </button>
                  {/if}
                </div>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
{/snippet}

<div class="h-full overflow-y-auto p-6 max-w-5xl mx-auto">
  <dl class="grid grid-cols-2 sm:grid-cols-4 gap-3 mb-6" aria-label="Usage summary" aria-busy={statsLoading}>
    <div class="rounded-lg border border-card-line px-3 py-2">
      <dt class="text-[11px] text-muted-foreground-1">Sessions</dt>
      <dd class="text-lg font-semibold text-foreground tabular-nums">{globalStats?.total_sessions ?? '--'}</dd>
    </div>
    <div class="rounded-lg border border-card-line px-3 py-2">
      <dt class="text-[11px] text-muted-foreground-1">AI requests</dt>
      <dd class="text-lg font-semibold text-foreground tabular-nums">{globalStats?.total_ai_requests ?? '--'}</dd>
    </div>
    <div class="rounded-lg border border-card-line px-3 py-2">
      <dt class="text-[11px] text-muted-foreground-1">Tokens</dt>
      <dd class="text-lg font-semibold text-foreground tabular-nums">{globalStats ? formatTokens(globalStats.total_input_tokens + globalStats.total_output_tokens) : '--'}</dd>
    </div>
    <div class="rounded-lg border border-card-line px-3 py-2">
      <dt class="text-[11px] text-muted-foreground-1">Estimated cost</dt>
      <dd class="text-lg font-semibold text-foreground tabular-nums">{globalStats ? formatCost(globalStats.total_estimated_cost) : '--'}</dd>
    </div>
  </dl>

  <ImageCatalog {ready} {prepareRuntime} />

  <!-- Action error banner -->
  {#if actionError}
    <div class="flex items-start gap-x-3 p-4 mb-4 rounded-lg border border-destructive/30 bg-destructive/10 text-sm">
      <Warning size={18} class="text-destructive mt-0.5 shrink-0" />
      <div class="flex-1 min-w-0">
        <p class="font-medium text-foreground">Session action failed</p>
        <p class="text-muted-foreground-1 mt-0.5 break-words">{actionError}</p>
      </div>
      <button
        type="button"
        class="shrink-0 size-6 inline-flex items-center justify-center rounded text-muted-foreground-1 hover:text-foreground"
        onclick={() => actionError = null}
        aria-label="Dismiss"
      >
        <X size={14} />
      </button>
    </div>
  {/if}

  <!-- Session list -->
  <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-3">Existing sessions</h3>
  {#if initialLoading}
    <div class="bg-card border border-card-line rounded-xl p-12 flex items-center justify-center gap-x-3">
      <CircleNotch size={18} class="text-muted-foreground-1 animate-spin" />
      <p class="text-muted-foreground-1 text-sm">Loading sessions...</p>
    </div>
  {:else if allVms.length === 0}
    <div class="bg-card border border-card-line rounded-xl p-8 flex items-center justify-center">
      <p class="text-muted-foreground-1 text-sm">No sessions</p>
    </div>
  {:else}
    {#if healthySessions.length > 0}
      {@render sessionTable(healthySessions)}
    {/if}
    {#if brokenSessions.length > 0}
      <div class="mt-6 flex items-center justify-between gap-x-3">
        <h3 class="text-xs font-semibold text-muted-foreground-1 uppercase tracking-wider">Broken sessions</h3>
        <button
          type="button"
          class="inline-flex items-center justify-center gap-x-2 rounded-lg bg-surface border border-line-2 px-3 py-1.5 text-xs font-medium text-foreground hover:bg-muted-hover focus:outline-hidden disabled:opacity-50 disabled:pointer-events-none"
          onclick={handlePurgeBroken}
          disabled={purging || vmStore.acting}
          title="Delete broken sessions"
        >
          <Trash size={14} />
          {purging ? 'Removing...' : 'Purge broken'}
        </button>
      </div>
      <div class="mt-3">
        {@render sessionTable(brokenSessions)}
      </div>
    {/if}
  {/if}


</div>

{#if credentialSession}
  {#key credentialSession.id}
    <SessionCredentials session={{id: credentialSession.id, name: credentialSession.name ?? undefined, running: credentialSession.status === 'Running'}} onclose={() => { credentialSession = null; }} />
  {/key}
{/if}

<Modal
  open={dashModalKind === 'stop'}
  title="Stop session"
  confirmLabel="Stop"
  destructive
  onconfirm={handleDashModalConfirm}
  oncancel={closeDashModal}
>
  <p class="text-sm text-foreground">Stop session <strong>{dashModalVm?.name ?? dashModalVm?.id}</strong>?</p>
</Modal>

<Modal
  open={dashModalKind === 'delete'}
  title="Delete session"
  confirmLabel="Delete"
  destructive
  onconfirm={handleDashModalConfirm}
  oncancel={closeDashModal}
>
  <p class="text-sm text-foreground">Delete session <strong>{dashModalVm?.name ?? dashModalVm?.id}</strong>? This cannot be undone.</p>
</Modal>
