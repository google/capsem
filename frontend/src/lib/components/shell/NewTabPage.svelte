<script lang="ts">
  import { onMount } from 'svelte';
  import { vmStore } from '../../stores/vms.svelte.ts';
  import { tabStore } from '../../stores/tabs.svelte.ts';
  import * as api from '../../api';
  import type { ProfileSummary } from '../../api';
  import type { AssetStatusResponse } from '../../types/assets';
  import type { VmSummary } from '../../types/gateway';
  import type { GlobalStats } from '../../types/gateway';
  import { formatUptime, formatTokens, formatCost } from '../../format';
  import { canOpenSession, hasVmAction, startAction, startLabel } from '../../vm-actions';
  import Modal from './Modal.svelte';
  import Pause from 'phosphor-svelte/lib/Pause';
  import Trash from 'phosphor-svelte/lib/Trash';
  import Play from 'phosphor-svelte/lib/Play';
  import Plus from 'phosphor-svelte/lib/Plus';
  import BracketsAngle from 'phosphor-svelte/lib/BracketsAngle';
  import CheckCircle from 'phosphor-svelte/lib/CheckCircle';
  import CircleNotch from 'phosphor-svelte/lib/CircleNotch';
  import DownloadSimple from 'phosphor-svelte/lib/DownloadSimple';
  import Warning from 'phosphor-svelte/lib/Warning';
  import X from 'phosphor-svelte/lib/X';
  import GitFork from 'phosphor-svelte/lib/GitFork';
  import Stop from 'phosphor-svelte/lib/Stop';

  type SortKey = 'name' | 'status' | 'profile' | 'uptime';
  type SortDir = 'asc' | 'desc';

  let globalStats = $state<GlobalStats | null>(null);
  let statsLoading = $state(true);

  let initialLoading = $derived(!vmStore.polled);

  type ProfileLauncher = {
    profile: ProfileSummary;
    assets: AssetStatusResponse | null;
    loading: boolean;
    ensuring: boolean;
    creating: boolean;
    error: string | null;
  };

  let profileLaunchers = $state<ProfileLauncher[]>([]);
  let profilesLoading = $state(true);
  let profilesError = $state<string | null>(null);

  onMount(async () => {
    void loadProfileLaunchers();
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

  function sortVms(list: VmSummary[]): VmSummary[] {
    return [...list].sort((a, b) => {
      let cmp = 0;
      switch (sortKey) {
        case 'name': cmp = (a.name ?? a.id).localeCompare(b.name ?? b.id); break;
        case 'status': cmp = a.status.localeCompare(b.status); break;
        case 'profile': cmp = a.profile_id.localeCompare(b.profile_id); break;
        case 'uptime': cmp = (a.uptime_secs ?? 0) - (b.uptime_secs ?? 0); break;
      }
      return sortDir === 'asc' ? cmp : -cmp;
    });
  }

  let allVms = $derived(sortVms(vmStore.vms));
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
    if (kind === 'stop') {
      await vmStore.stop(id);
    } else if (kind === 'delete') {
      const tab = tabStore.tabs.find(t => t.vmId === id);
      if (tab) tabStore.close(tab.id);
      await vmStore.delete(id);
    }
  }

  async function handleStart(e: MouseEvent, vm: VmSummary) {
    e.stopPropagation();
    if (!hasVmAction(vm, startAction(vm))) {
      actionError = vm.resume_blocked_reason ?? `${vm.name ?? vm.id} cannot be resumed.`;
      return;
    }
    await vmStore.resume(vm.name ?? vm.id);
  }

  async function handlePause(e: MouseEvent, vm: VmSummary) {
    e.stopPropagation();
    await vmStore.suspend(vm.id);
  }

  async function handleFork(e: MouseEvent, vm: VmSummary) {
    e.stopPropagation();
    const baseName = vm.name ?? vm.id;
    const name = prompt('Fork name:', `${baseName}-fork`);
    if (name?.trim()) await vmStore.fork(vm.id, { name: name.trim() });
  }

  let creatingVm = $state(false);
  let actionError = $state<string | null>(null);

  function profileAssetText(assetHealth: AssetStatusResponse | null): string {
    if (!assetHealth) return 'Checking profile assets.';
    if (assetHealth.downloading) {
      const name = assetHealth.current_asset ? ` ${assetHealth.current_asset}` : '';
      if (assetHealth.bytes_total && assetHealth.bytes_total > 0) {
        const pct = Math.floor(((assetHealth.bytes_done ?? 0) / assetHealth.bytes_total) * 100);
        return `Downloading${name}: ${pct}%`;
      }
      return `Downloading${name}.`;
    }
    if (assetHealth.error || assetHealth.reconcile_error) {
      return assetHealth.error ?? assetHealth.reconcile_error ?? 'Asset reconciliation failed.';
    }
    const missingAssets = assetHealth.assets
      .filter(asset => asset.status !== 'present')
      .map(asset => asset.name);
    if (missingAssets.length > 0) return `Missing: ${missingAssets.join(', ')}.`;
    return assetHealth.ready ? 'Ready.' : 'Assets are not ready.';
  }

  function profileAssetChecklist(launcher: ProfileLauncher) {
    return launcher.assets?.assets.slice(0, 4) ?? [];
  }

  function updateProfileLauncher(profileId: string, patch: Partial<ProfileLauncher>) {
    profileLaunchers = profileLaunchers.map(launcher =>
      launcher.profile.id === profileId ? { ...launcher, ...patch } : launcher
    );
  }

  function delay(ms: number): Promise<void> {
    return new Promise(resolve => window.setTimeout(resolve, ms));
  }

  async function fetchProfileAssets(profile: ProfileSummary): Promise<ProfileLauncher> {
    try {
      return {
        profile,
        assets: await api.getAssetsStatus(profile.id),
        loading: false,
        ensuring: false,
        creating: false,
        error: null,
      };
    } catch (err) {
      return {
        profile,
        assets: null,
        loading: false,
        ensuring: false,
        creating: false,
        error: parseApiError(err),
      };
    }
  }

  async function loadProfileLaunchers() {
    profilesLoading = true;
    profilesError = null;
    try {
      const profiles = (await api.listProfiles()).profiles.filter(profile => profile.availability.web);
      profileLaunchers = profiles.map(profile => ({
        profile,
        assets: null,
        loading: true,
        ensuring: false,
        creating: false,
        error: null,
      }));
      profileLaunchers = await Promise.all(profiles.map(fetchProfileAssets));
    } catch (err) {
      profilesError = parseApiError(err);
      profileLaunchers = [];
    } finally {
      profilesLoading = false;
    }
  }

  async function refreshDashboard() {
    statsLoading = true;
    await Promise.all([
      vmStore.refresh(),
      loadProfileLaunchers(),
      api.getStats()
        .then(stats => { globalStats = stats.global; })
        .catch(() => { globalStats = null; }),
    ]);
    statsLoading = false;
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

  async function createFromProfile(profileId: string) {
    if (creatingVm) return;
    actionError = null;
    const launcher = profileLaunchers.find(item => item.profile.id === profileId);
    if (!launcher || launcher.assets?.ready !== true) {
      actionError = `Assets are not ready for profile ${profileId}`;
      return;
    }
    creatingVm = true;
    updateProfileLauncher(profileId, { creating: true });
    try {
      const { id, name } = await vmStore.provision({
        profile_id: profileId,
        ram_mb: 2048,
        cpus: 2,
        persistent: true,
      });
      console.log('[NewTabPage] provision OK id=%s name=%s', id, name);
      tabStore.openVM(id, name);
    } catch (e) {
      console.error('[NewTabPage] provision FAIL:', e);
      actionError = parseApiError(e);
    } finally {
      creatingVm = false;
      updateProfileLauncher(profileId, { creating: false });
    }
  }

  async function ensureProfileAssets(profileId: string) {
    actionError = null;
    updateProfileLauncher(profileId, { ensuring: true, error: null });
    try {
      let assets = await api.ensureAssets(profileId);
      updateProfileLauncher(profileId, { assets });
      for (let attempt = 0; attempt < 120 && assets.downloading && !assets.ready; attempt += 1) {
        await delay(1000);
        assets = await api.getAssetsStatus(profileId);
        updateProfileLauncher(profileId, { assets });
        if (assets.ready || !assets.downloading) break;
      }
      updateProfileLauncher(profileId, { assets, ensuring: false });
      await vmStore.refresh();
    } catch (err) {
      updateProfileLauncher(profileId, { ensuring: false, error: parseApiError(err) });
    }
  }

  function openCustomizeProfile(profileId: string) {
    vmStore.openCreateModal(profileId);
  }

  async function handlePurgeBroken() {
    actionError = null;
    try {
      await api.purge();
      await vmStore.refresh();
    } catch (err) {
      actionError = parseApiError(err);
    }
  }
</script>

{#snippet sessionTable(vms: VmSummary[])}
  <div class="flex flex-col">
    <div class="max-h-[50vh] overflow-y-auto overflow-x-auto [&::-webkit-scrollbar]:h-2 [&::-webkit-scrollbar]:w-2 [&::-webkit-scrollbar-thumb]:rounded-none [&::-webkit-scrollbar-track]:bg-scrollbar-track [&::-webkit-scrollbar-thumb]:bg-scrollbar-thumb">
      <table class="min-w-full">
        <thead class="border-b border-table-line">
          <tr>
            {#each [
              { key: 'name', label: 'Name' },
              { key: 'status', label: 'Status' },
              { key: 'profile', label: 'Profile' },
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
            <th scope="col" class="py-2 px-3 text-end font-normal text-sm text-muted-foreground-1">Actions</th>
          </tr>
        </thead>

        <tbody class="divide-y divide-table-line">
          {#each vms as vm (vm.id)}
            <tr
              class="{canOpenSession(vm) ? 'hover:bg-muted-hover cursor-pointer' : 'opacity-60 cursor-default'}"
              onclick={() => { if (canOpenSession(vm)) tabStore.openVM(vm.id, vm.name ?? vm.id); }}
            >
              <td class="p-3 whitespace-nowrap text-sm font-medium text-foreground">{vm.name ?? vm.id}</td>
              <td class="p-3 whitespace-nowrap text-sm">
                <span class="text-xs px-2 py-0.5 rounded-full {statusBadge(vm.status)}">{vm.status}</span>
              </td>
              <td class="p-3 whitespace-nowrap text-sm text-muted-foreground-1">{vm.profile_id}</td>
              <td class="p-3 whitespace-nowrap text-sm text-muted-foreground-1 tabular-nums">{vm.uptime_secs != null ? formatUptime(vm.uptime_secs) : '--'}</td>
              <td class="p-3 whitespace-nowrap text-sm text-muted-foreground-1 tabular-nums">{vm.total_input_tokens != null ? formatTokens((vm.total_input_tokens ?? 0) + (vm.total_output_tokens ?? 0)) : '--'}</td>
              <td class="p-3 whitespace-nowrap text-sm text-muted-foreground-1 tabular-nums">{vm.total_estimated_cost != null ? formatCost(vm.total_estimated_cost) : '--'}</td>
              <td class="p-3 whitespace-nowrap text-end">
                <div class="inline-flex items-center gap-x-1">
                  {#if hasVmAction(vm, 'pause')}
                    <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-surface" onclick={(e: MouseEvent) => handlePause(e, vm)} aria-label="Pause" title="Pause">
                      <Pause size={16} />
                    </button>
                  {/if}
                  {#if hasVmAction(vm, 'stop')}
                    <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-surface" onclick={(e: MouseEvent) => openDashModal(e, 'stop', vm)} aria-label="Stop" title="Stop">
                      <Stop size={16} />
                    </button>
                  {/if}
                  {#if hasVmAction(vm, startAction(vm))}
                    <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-primary hover:bg-surface" onclick={(e: MouseEvent) => handleStart(e, vm)} aria-label={startLabel(vm)} title={startLabel(vm)}>
                      <Play size={16} />
                    </button>
                  {/if}
                  {#if hasVmAction(vm, 'fork')}
                    <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-surface" onclick={(e: MouseEvent) => handleFork(e, vm)} aria-label="Fork" title="Fork">
                      <GitFork size={16} />
                    </button>
                  {/if}
                  {#if hasVmAction(vm, 'delete')}
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
  <!-- Sessions header -->
  <div class="flex items-center justify-between mb-6">
    <h2 class="text-2xl font-bold text-foreground">Sessions</h2>
    <button
      type="button"
      class="inline-flex items-center justify-center gap-x-2 rounded-lg bg-surface border border-line-2 px-3 py-1.5 text-xs font-medium text-foreground hover:bg-muted-hover focus:outline-hidden disabled:opacity-50 disabled:pointer-events-none"
      onclick={refreshDashboard}
      disabled={profilesLoading || statsLoading || vmStore.acting}
      title="Refresh dashboard"
    >
      Refresh
    </button>
  </div>

  <!-- Profile launchers -->
  <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-3">Start from a profile</h3>
  {#if profilesLoading}
    <div class="bg-card border border-card-line rounded-xl p-6 flex items-center gap-x-3 mb-6">
      <CircleNotch size={18} class="text-muted-foreground-1 animate-spin" />
      <p class="text-muted-foreground-1 text-sm">Loading profiles...</p>
    </div>
  {:else if profilesError}
    <div class="flex items-start gap-x-3 p-4 mb-6 rounded-lg border border-destructive/30 bg-destructive/10 text-sm">
      <Warning size={18} class="text-destructive mt-0.5 shrink-0" />
      <div class="flex-1 min-w-0">
        <p class="font-medium text-foreground">Profiles unavailable</p>
        <p class="text-muted-foreground-1 mt-0.5 break-words">{profilesError}</p>
      </div>
      <button
        type="button"
        class="shrink-0 inline-flex items-center gap-x-2 bg-layer border border-layer-line text-layer-foreground hover:bg-muted-hover rounded-lg px-3 py-1.5 text-xs font-medium"
        onclick={loadProfileLaunchers}
      >
        Retry
      </button>
    </div>
  {:else if profileLaunchers.length === 0}
    <div class="bg-card border border-card-line rounded-xl p-6 flex items-center justify-center mb-6">
      <p class="text-muted-foreground-1 text-sm">No web-available profiles</p>
    </div>
  {:else}
    <div class="grid grid-cols-1 md:grid-cols-2 gap-3 mb-6">
      {#each profileLaunchers as launcher (launcher.profile.id)}
        {@const ready = launcher.assets?.ready === true}
        {@const busy = launcher.loading || launcher.ensuring || launcher.creating || launcher.assets?.downloading === true}
        <div class="group bg-card border border-card-line rounded-xl p-4 transition-colors hover:border-primary/50 hover:bg-muted-hover">
          <div class="flex items-start gap-x-3">
            <span class="size-10 shrink-0 inline-flex items-center justify-center rounded-lg bg-muted text-foreground [&>svg]:size-5 [&>svg]:max-w-5 [&>svg]:max-h-5" aria-hidden="true">
              {#if launcher.profile.icon_svg}
                {@html launcher.profile.icon_svg}
              {:else}
                <BracketsAngle size={20} weight="bold" />
              {/if}
            </span>
            <span class="min-w-0 flex-1">
              <span class="flex items-center gap-x-3">
                <span class="text-sm font-semibold text-foreground truncate">{launcher.profile.name}</span>
              </span>
              <span class="block text-xs text-muted-foreground-1 mt-1 line-clamp-2">{launcher.profile.description}</span>
              <span class="block text-[11px] text-muted-foreground-2 mt-2">{launcher.error ?? profileAssetText(launcher.assets)}</span>
              {#if profileAssetChecklist(launcher).length > 0}
                <span class="mt-3 block">
                  <span class="block text-[11px] font-semibold uppercase tracking-wider text-muted-foreground-2">VM assets</span>
                  <span class="mt-1 grid gap-1">
                    {#each profileAssetChecklist(launcher) as asset (`${asset.arch ?? ''}:${asset.kind ?? asset.name}`)}
                      <span class="flex items-center gap-x-1.5 text-[11px] text-muted-foreground-1">
                        {#if asset.status === 'present'}
                          <CheckCircle size={12} weight="fill" class="text-primary shrink-0" />
                        {:else if asset.status === 'downloading'}
                          <CircleNotch size={12} class="text-muted-foreground-1 animate-spin shrink-0" />
                        {:else}
                          <Warning size={12} class="text-destructive shrink-0" />
                        {/if}
                        <span class="truncate">{asset.kind ?? asset.name}</span>
                      </span>
                    {/each}
                  </span>
                </span>
              {/if}
              <span class="mt-3 flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  class="inline-flex items-center justify-center gap-x-2 rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary-hover focus:outline-hidden focus:bg-primary-focus disabled:opacity-50 disabled:pointer-events-none"
                  onclick={() => ready ? createFromProfile(launcher.profile.id) : ensureProfileAssets(launcher.profile.id)}
                  disabled={creatingVm || launcher.loading || launcher.creating || launcher.ensuring || launcher.assets?.downloading === true}
                  title={ready ? `New ${launcher.profile.name} session` : profileAssetText(launcher.assets)}
                >
                  {#if ready}
                    <Plus size={14} weight="bold" />
                    New
                  {:else}
                    <DownloadSimple size={14} />
                    Download
                  {/if}
                </button>
                <button
                  type="button"
                  class="inline-flex items-center justify-center gap-x-2 rounded-lg bg-surface border border-line-2 px-3 py-1.5 text-xs font-medium text-foreground hover:bg-muted-hover focus:outline-hidden disabled:opacity-50 disabled:pointer-events-none"
                  onclick={() => openCustomizeProfile(launcher.profile.id)}
                  disabled={creatingVm}
                  title={`Customize ${launcher.profile.name} session`}
                >
                  <Plus size={14} weight="bold" />
                  Customize
                </button>
              </span>
            </span>
          </div>
        </div>
      {/each}
    </div>
  {/if}

  <!-- Action error banner -->
  {#if actionError}
    <div class="flex items-start gap-x-3 p-4 mb-4 rounded-lg border border-destructive/30 bg-destructive/10 text-sm">
      <Warning size={18} class="text-destructive mt-0.5 shrink-0" />
      <div class="flex-1 min-w-0">
        <p class="font-medium text-foreground">Failed to create session</p>
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
  <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-3">Sessions</h3>
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
          disabled={vmStore.acting}
          title="Purge temporary and broken sessions"
        >
          <Trash size={14} />
          Purge broken
        </button>
      </div>
      <div class="mt-3">
        {@render sessionTable(brokenSessions)}
      </div>
    {/if}
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
