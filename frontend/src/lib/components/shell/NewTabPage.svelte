<script lang="ts">
  import { onMount } from 'svelte';
  import { vmStore } from '../../stores/vms.svelte.ts';
  import { tabStore } from '../../stores/tabs.svelte.ts';
  import * as api from '../../api';
  import type { VmProfileStatus, VmSummary } from '../../types/gateway';
  import type { GlobalStats } from '../../types/gateway';
  import { formatUptime, formatTokens, formatCost, formatBytes } from '../../format';
  import Modal from './Modal.svelte';
  import ArrowClockwise from 'phosphor-svelte/lib/ArrowClockwise';
  import Pause from 'phosphor-svelte/lib/Pause';
  import Trash from 'phosphor-svelte/lib/Trash';
  import Play from 'phosphor-svelte/lib/Play';
  import Plus from 'phosphor-svelte/lib/Plus';
  import BracketsAngle from 'phosphor-svelte/lib/BracketsAngle';
  import CircleNotch from 'phosphor-svelte/lib/CircleNotch';
  import Warning from 'phosphor-svelte/lib/Warning';
  import X from 'phosphor-svelte/lib/X';
  import GitFork from 'phosphor-svelte/lib/GitFork';
  import FloppyDisk from 'phosphor-svelte/lib/FloppyDisk';

  type SortKey = 'name' | 'status' | 'profile' | 'uptime' | 'tokens' | 'cost';
  type SortDir = 'asc' | 'desc';

  let globalStats = $state<GlobalStats | null>(null);
  let statsLoading = $state(true);

  let initialLoading = $derived(!vmStore.polled);

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

  function sortVms(list: VmSummary[]): VmSummary[] {
    return [...list].sort((a, b) => {
      let cmp = 0;
      switch (sortKey) {
        case 'name': cmp = (a.name ?? a.id).localeCompare(b.name ?? b.id); break;
        case 'status': cmp = a.status.localeCompare(b.status); break;
        case 'profile': cmp = profileSortValue(a).localeCompare(profileSortValue(b)); break;
        case 'uptime': cmp = (a.uptime_secs ?? 0) - (b.uptime_secs ?? 0); break;
        case 'tokens': cmp = ((a.total_input_tokens ?? 0) + (a.total_output_tokens ?? 0)) - ((b.total_input_tokens ?? 0) + (b.total_output_tokens ?? 0)); break;
        case 'cost': cmp = (a.total_estimated_cost ?? 0) - (b.total_estimated_cost ?? 0); break;
      }
      return sortDir === 'asc' ? cmp : -cmp;
    });
  }

  let ephemeralVms = $derived(sortVms(vmStore.vms.filter(v => !v.persistent)));
  let persistentVms = $derived(sortVms(vmStore.vms.filter(v => v.persistent)));

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

  const profileStatusColor: Record<VmProfileStatus, string> = {
    current: 'bg-primary text-primary-foreground',
    needs_update: 'border border-warning/40 bg-warning/10 text-warning',
    deprecated: 'border border-warning/40 bg-warning/10 text-warning',
    revoked: 'border border-destructive/40 bg-destructive/10 text-destructive',
    corrupted: 'border border-destructive/40 bg-destructive/10 text-destructive',
    unknown: 'border border-line-2 bg-muted text-muted-foreground-1',
  };

  function resolvedProfileStatus(vm: VmSummary): VmProfileStatus {
    if (!vm.profile_id) return 'corrupted';
    return vm.profile_status ?? 'unknown';
  }

  function profileStatusBadge(vm: VmSummary): string {
    return profileStatusColor[resolvedProfileStatus(vm)];
  }

  function profileStatusLabel(vm: VmSummary): string {
    return resolvedProfileStatus(vm).replace('_', ' ');
  }

  function profileIdentity(vm: VmSummary): string {
    if (!vm.profile_id) return 'missing profile';
    return vm.profile_revision ? `${vm.profile_id}@${vm.profile_revision}` : vm.profile_id;
  }

  function profileSortValue(vm: VmSummary): string {
    return `${profileIdentity(vm)}:${resolvedProfileStatus(vm)}`;
  }

  function shortHash(value: string | null | undefined): string {
    if (!value) return 'none';
    return value.length > 32 ? `${value.slice(0, 28)}...` : value;
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
    if (kind === 'stop') {
      await vmStore.stop(id);
    } else if (kind === 'destroy') {
      const tab = tabStore.tabs.find(t => t.vmId === id);
      if (tab) tabStore.close(tab.id);
      await vmStore.delete(id);
    }
  }

  async function handleResume(e: MouseEvent, vm: VmSummary) {
    e.stopPropagation();
    if (vm.name) await vmStore.resume(vm.name);
  }

  let creatingTemp = $state(false);
  let actionError = $state<string | null>(null);
  let setupRetrying = $state(false);
  let setupRetryError = $state<string | null>(null);

  let serviceReady = $derived(vmStore.serviceStatus === 'running');
  let assetsReady = $derived(vmStore.assetHealth?.ready === true);
  let canCreateSessions = $derived(serviceReady && assetsReady);
  let startupBlocked = $derived(!initialLoading && !canCreateSessions);
  let readyProfileAssets = $derived(
    vmStore.assetHealth?.ready === true && vmStore.assetHealth.profile_id ? vmStore.assetHealth : null,
  );
  let assetStatus = $derived.by(() => {
    if (!serviceReady) {
      return {
        title: 'Capsem service is offline',
        message: 'Start or recover the service before creating sessions.',
        details: [] as string[],
        showRetry: false,
        severity: 'error' as const,
      };
    }

    if (!vmStore.assetHealth) {
      return {
        title: 'VM asset status is unknown',
        message: 'Waiting for the service to report rootfs and manifest readiness.',
        details: [] as string[],
        showRetry: false,
        severity: 'warning' as const,
      };
    }

    const savedVmDependencyDetails = (vmStore.assetHealth.saved_vm_dependencies ?? []).map(dep =>
      `${dep.vm} missing ${dep.missing.join(', ')} (${dep.recovery_hint})`,
    );

    if (!vmStore.assetHealth.ready) {
      if (vmStore.assetHealth.state === 'checking') {
        return {
          title: 'VM assets are being checked',
          message: 'Waiting for the service to verify rootfs and manifest readiness.',
          details: savedVmDependencyDetails,
          showRetry: false,
          severity: 'warning' as const,
        };
      }
      if (vmStore.assetHealth.state === 'updating') {
        const progress = vmStore.assetHealth.progress
          ? `Updating ${vmStore.assetHealth.progress.logical_name}.`
          : 'Required VM assets are updating.';
        return {
          title: 'VM assets are updating',
          message: progress,
          details: savedVmDependencyDetails,
          showRetry: false,
          severity: 'warning' as const,
        };
      }
      if (vmStore.assetHealth.state === 'error') {
        return {
          title: 'VM assets need attention',
          message: vmStore.assetHealth.error ?? 'The service could not prepare required VM assets.',
          details: savedVmDependencyDetails,
          showRetry: vmStore.assetHealth.retryable,
          severity: 'error' as const,
        };
      }
      const missing = vmStore.assetHealth.missing.length > 0
        ? `Missing: ${vmStore.assetHealth.missing.join(', ')}.`
        : 'Required VM assets are not ready.';
      return {
        title: 'VM assets are missing',
        message: `${missing} The service will keep trying in the background.`,
        details: savedVmDependencyDetails,
        showRetry: false,
        severity: 'warning' as const,
      };
    }
    return null;
  });

  function emptySessionText(kind: 'ephemeral' | 'persistent'): string {
    if (startupBlocked) {
      return 'Session list unavailable until startup checks pass';
    }
    return kind === 'ephemeral' ? 'No ephemeral sessions' : 'No persistent sessions';
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

  async function createTemporary() {
    console.log('[NewTabPage] createTemporary() creatingTemp=%s', creatingTemp);
    if (creatingTemp) return;
    actionError = null;
    creatingTemp = true;
    try {
      console.log('[NewTabPage] calling vmStore.provision()');
      const request = {
        persistent: false,
        ...profileProvisionFields(),
      };
      const { id, name } = await vmStore.provision(request);
      console.log('[NewTabPage] provision OK id=%s name=%s', id, name);
      tabStore.openVM(id, name);
    } catch (e) {
      console.error('[NewTabPage] provision FAIL:', e);
      actionError = parseApiError(e);
    } finally {
      creatingTemp = false;
    }
  }

  function profileProvisionFields(): { profile_id?: string; profile_revision?: string } {
    const profileId = vmStore.assetHealth?.profile_id;
    if (!profileId) return {};
    return {
      profile_id: profileId,
      ...(vmStore.assetHealth?.profile_revision ? { profile_revision: vmStore.assetHealth.profile_revision } : {}),
    };
  }

  async function retrySetup(): Promise<void> {
    if (setupRetrying) return;
    setupRetrying = true;
    setupRetryError = null;
    try {
      await api.retrySetup();
      await vmStore.refresh();
    } catch (e) {
      setupRetryError = parseApiError(e);
    } finally {
      setupRetrying = false;
    }
  }
</script>

{#snippet sessionTable(vms: VmSummary[])}
  <div class="flex flex-col">
    <div class="overflow-x-auto [&::-webkit-scrollbar]:h-2 [&::-webkit-scrollbar-thumb]:rounded-none [&::-webkit-scrollbar-track]:bg-scrollbar-track [&::-webkit-scrollbar-thumb]:bg-scrollbar-thumb">
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
            <tr class="hover:bg-muted-hover cursor-pointer" onclick={() => tabStore.openVM(vm.id, vm.name ?? vm.id)}>
              <td class="p-3 whitespace-nowrap text-sm font-medium text-foreground">{vm.name ?? vm.id}</td>
              <td class="p-3 whitespace-nowrap text-sm">
                <span class="text-xs px-2 py-0.5 rounded-full {statusBadge(vm.status)}">{vm.status}</span>
              </td>
              <td class="p-3 whitespace-nowrap text-sm">
                <div class="flex flex-col gap-y-1">
                  <span class="font-mono text-xs text-foreground">{profileIdentity(vm)}</span>
                  <span class="w-fit text-[10px] px-1.5 py-0.5 rounded-full {profileStatusBadge(vm)}">
                    {profileStatusLabel(vm)}
                  </span>
                </div>
              </td>
              <td class="p-3 whitespace-nowrap text-sm text-muted-foreground-1 tabular-nums">{vm.uptime_secs != null ? formatUptime(vm.uptime_secs) : '--'}</td>
              <td class="p-3 whitespace-nowrap text-sm text-muted-foreground-1 tabular-nums">{vm.total_input_tokens != null ? formatTokens((vm.total_input_tokens ?? 0) + (vm.total_output_tokens ?? 0)) : '--'}</td>
              <td class="p-3 whitespace-nowrap text-sm text-muted-foreground-1 tabular-nums">{vm.total_estimated_cost != null ? formatCost(vm.total_estimated_cost) : '--'}</td>
              <td class="p-3 whitespace-nowrap text-end">
                <div class="inline-flex items-center gap-x-1">
                  {#if !vm.persistent}
                    <!-- Ephemeral: save (persist) + destroy -->
                    {#if vm.status === 'Running'}
                      <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-primary hover:bg-surface" onclick={async (e: MouseEvent) => { e.stopPropagation(); const name = prompt('Save as:'); if (name) await vmStore.persist(vm.id, name); }} aria-label="Save" title="Save as persistent">
                        <FloppyDisk size={16} />
                      </button>
                    {/if}
                    <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-destructive hover:bg-surface" onclick={(e: MouseEvent) => openDashModal(e, 'destroy', vm)} aria-label="Destroy" title="Destroy">
                      <Trash size={16} />
                    </button>
                  {:else}
                    <!-- Persistent: actions depend on status -->
                    {#if vm.status === 'Running'}
                      <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-surface" onclick={async (e: MouseEvent) => { e.stopPropagation(); await vmStore.restart(vm.id); }} aria-label="Restart" title="Restart">
                        <ArrowClockwise size={16} />
                      </button>
                      <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-surface" onclick={async (e: MouseEvent) => { e.stopPropagation(); await vmStore.suspend(vm.id); }} aria-label="Pause" title="Pause">
                        <Pause size={16} />
                      </button>
                      <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-surface" onclick={async (e: MouseEvent) => { e.stopPropagation(); const name = prompt('Fork name:'); if (name) await vmStore.fork(vm.id, { name }); }} aria-label="Fork" title="Fork">
                        <GitFork size={16} />
                      </button>
                    {:else if vm.status === 'Stopped' || vm.status === 'Suspended' || vm.status === 'Error'}
                      <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-primary hover:bg-surface" onclick={(e: MouseEvent) => handleResume(e, vm)} aria-label="Resume" title="Resume">
                        <Play size={16} />
                      </button>
                    {/if}
                    <button type="button" class="size-7 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-destructive hover:bg-surface" onclick={(e: MouseEvent) => openDashModal(e, 'destroy', vm)} aria-label="Delete" title="Delete">
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

<div class="p-6 max-w-5xl mx-auto">
  <!-- Sessions header -->
  <div class="flex items-center justify-between mb-6">
    <h2 class="text-2xl font-bold text-foreground">Sessions</h2>
    <div class="flex items-center gap-x-2">
      <button
        type="button"
        class="inline-flex items-center gap-x-2 bg-surface border border-line-2 text-foreground hover:bg-muted-hover rounded-lg px-4 py-2 text-sm font-medium transition-colors disabled:opacity-50 disabled:pointer-events-none"
        onclick={() => vmStore.showCreateModal = true}
        disabled={creatingTemp || !canCreateSessions}
      >
        <Plus size={16} weight="bold" />
        Customize Session...
      </button>
      <button
        type="button"
        class="inline-flex items-center gap-x-2 bg-primary text-primary-foreground hover:bg-primary-hover rounded-lg px-4 py-2 text-sm font-medium transition-colors disabled:opacity-50 disabled:pointer-events-none"
        onclick={createTemporary}
        disabled={creatingTemp || !canCreateSessions}
      >
        <BracketsAngle size={16} weight="bold" />
        {creatingTemp ? 'Creating...' : 'Quick Session'}
      </button>
    </div>
  </div>

  <!-- Asset health warning -->
  {#if assetStatus}
    <div class="flex items-start gap-x-3 p-4 mb-4 rounded-lg text-sm border {assetStatus.severity === 'error' ? 'border-destructive/30 bg-destructive/10' : 'border-warning/30 bg-warning/10'}">
      <Warning size={18} class="{assetStatus.severity === 'error' ? 'text-destructive' : 'text-warning'} mt-0.5 shrink-0" />
      <div>
        <p class="font-medium text-foreground">{assetStatus.title}</p>
        <p class="text-muted-foreground-1 mt-0.5">
          {assetStatus.message}
        </p>
        {#if assetStatus.details.length > 0}
          <ul class="mt-2 space-y-1">
            {#each assetStatus.details as detail}
              <li class="text-xs text-muted-foreground">{detail}</li>
            {/each}
          </ul>
        {/if}
        <div class="mt-3 flex items-center gap-2">
          {#if assetStatus.showRetry}
            <button
              type="button"
              class="py-1 px-3 text-xs font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              disabled={setupRetrying}
              onclick={retrySetup}
            >
              {setupRetrying ? 'Retrying...' : 'Retry setup'}
            </button>
          {/if}
          <button
            type="button"
            class="py-1 px-3 text-xs font-medium rounded-lg bg-layer border border-layer-line text-layer-foreground hover:bg-layer-hover transition-colors"
            onclick={() => vmStore.refresh()}
          >
            Refresh status
          </button>
        </div>
        {#if setupRetryError}
          <p class="mt-2 text-xs text-destructive">{setupRetryError}</p>
        {/if}
      </div>
    </div>
  {/if}

  {#if readyProfileAssets}
    <section class="mb-4 rounded-lg border border-line-2 bg-layer p-4">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider">Profile Assets</h3>
          <p class="mt-1 font-mono text-sm text-foreground">
            {readyProfileAssets.profile_revision ? `${readyProfileAssets.profile_id}@${readyProfileAssets.profile_revision}` : readyProfileAssets.profile_id}
          </p>
        </div>
        <span class="rounded-full bg-primary px-2 py-0.5 text-xs text-primary-foreground">ready</span>
      </div>
      <dl class="mt-3 grid gap-3 text-xs sm:grid-cols-3">
        <div>
          <dt class="text-muted-foreground-1">Arch</dt>
          <dd class="mt-1 font-mono text-foreground">{readyProfileAssets.arch ?? 'unknown'}</dd>
        </div>
        <div>
          <dt class="text-muted-foreground-1">Asset version</dt>
          <dd class="mt-1 font-mono text-foreground">{readyProfileAssets.version ?? 'unknown'}</dd>
        </div>
        <div>
          <dt class="text-muted-foreground-1">Payload hash</dt>
          <dd class="mt-1 font-mono text-foreground" title={readyProfileAssets.profile_payload_hash ?? undefined}>{shortHash(readyProfileAssets.profile_payload_hash)}</dd>
        </div>
      </dl>
      {#if (readyProfileAssets.profile_assets ?? []).length > 0}
        <div class="mt-3 overflow-x-auto">
          <table class="min-w-full text-xs">
            <thead class="border-b border-table-line text-muted-foreground-1">
              <tr>
                <th scope="col" class="py-1 pr-3 text-left font-normal">Asset</th>
                <th scope="col" class="py-1 pr-3 text-left font-normal">Size</th>
                <th scope="col" class="py-1 pr-3 text-left font-normal">Hash</th>
                <th scope="col" class="py-1 text-left font-normal">Source</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-table-line">
              {#each readyProfileAssets.profile_assets ?? [] as asset (asset.logical_name)}
                <tr>
                  <td class="py-1.5 pr-3 font-mono text-foreground">{asset.logical_name}</td>
                  <td class="py-1.5 pr-3 font-mono text-muted-foreground-1">{formatBytes(asset.size)}</td>
                  <td class="py-1.5 pr-3 font-mono text-muted-foreground-1" title={asset.hash}>{shortHash(asset.hash)}</td>
                  <td class="py-1.5 font-mono text-muted-foreground-1 break-all">{asset.source_url}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </section>
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

  <!-- Ephemeral sessions -->
  <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-3">Ephemeral</h3>
  {#if initialLoading}
    <div class="bg-card border border-card-line rounded-xl p-12 flex items-center justify-center gap-x-3">
      <CircleNotch size={18} class="text-muted-foreground-1 animate-spin" />
      <p class="text-muted-foreground-1 text-sm">Loading sessions...</p>
    </div>
  {:else if ephemeralVms.length === 0}
    <div class="bg-card border border-card-line rounded-xl p-8 flex items-center justify-center">
      <p class="text-muted-foreground-1 text-sm">{emptySessionText('ephemeral')}</p>
    </div>
  {:else}
    {@render sessionTable(ephemeralVms)}
  {/if}

  <!-- Persistent sessions -->
  <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mt-8 mb-3">Persistent</h3>
  {#if initialLoading}
    <div class="flex items-center gap-x-2 py-3">
      <CircleNotch size={14} class="text-muted-foreground-1 animate-spin" />
      <span class="text-xs text-muted-foreground-1">Loading...</span>
    </div>
  {:else if persistentVms.length === 0}
    <div class="bg-card border border-card-line rounded-xl p-8 flex items-center justify-center">
      <p class="text-muted-foreground-1 text-sm">{emptySessionText('persistent')}</p>
    </div>
  {:else}
    {@render sessionTable(persistentVms)}
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
