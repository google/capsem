<script lang="ts">
  import { onMount } from 'svelte';
  import * as api from '../../api';
  import { vmStore } from '../../stores/vms.svelte.ts';
  import { tabStore } from '../../stores/tabs.svelte.ts';
  import type { ProfileListRecord, ProvisionRequest } from '../../types/gateway';
  import Modal from './Modal.svelte';
  import BracketsAngle from 'phosphor-svelte/lib/BracketsAngle';
  import Briefcase from 'phosphor-svelte/lib/Briefcase';
  import WarningCircle from 'phosphor-svelte/lib/WarningCircle';

  let name = $state('');
  let profiles = $state<ProfileListRecord[]>([]);
  let selectedProfileId = $state('');
  let profileError = $state<string | null>(null);
  let loadingProfiles = $state(false);
  let resourceMode = $state<'service' | 'custom'>('service');
  let ramMb = $state(8192);
  let cpus = $state(4);
  let error = $state<string | null>(null);
  let creating = $state(false);
  let serviceReady = $derived(vmStore.serviceStatus === 'running');
  let selectedProfile = $derived(profiles.find(profile => profile.profile.id === selectedProfileId) ?? null);
  let selectedProfileReady = $derived(Boolean(selectedProfile && profileUsable(selectedProfile)));
  let createDisabled = $derived(creating || loadingProfiles || !serviceReady || !selectedProfileReady);

  onMount(() => {
    loadProfiles();
  });

  async function loadProfiles() {
    loadingProfiles = true;
    profileError = null;
    try {
      const response = await api.listProfiles();
      profiles = response.profiles;
      selectedProfileId =
        response.default_profile && response.profiles.some(profile => profile.profile.id === response.default_profile)
          ? response.default_profile
          : (response.profiles.find(profileUsable)?.profile.id ?? response.profiles[0]?.profile.id ?? '');
    } catch (e: any) {
      profileError = e.message || 'Failed to load profiles';
      profiles = [];
      selectedProfileId = '';
    } finally {
      loadingProfiles = false;
    }
  }

  function close() {
    vmStore.showCreateModal = false;
    name = '';
    selectedProfileId = '';
    profileError = null;
    resourceMode = 'service';
    ramMb = 8192;
    cpus = 4;
    error = null;
  }

  async function handleSubmit() {
    if (creating) return;
    error = null;
    creating = true;
    try {
      await vmStore.refresh();
      if (vmStore.serviceStatus !== 'running') {
        error = 'Capsem service is not running.';
        return;
      }
      if (!selectedProfile || !profileUsable(selectedProfile)) {
        error = 'Choose a ready profile before starting a session.';
        return;
      }
      const hasName = name.trim().length > 0;
      const request: ProvisionRequest = {
        persistent: hasName,
        ...(hasName ? { name: name.trim() } : {}),
        ...profileProvisionFields(),
      };
      if (resourceMode === 'custom') {
        request.ram_mb = ramMb;
        request.cpus = cpus;
      }
      const { id, name: finalName } = await vmStore.provision(request);
      tabStore.openVM(id, finalName);
      close();
    } catch (e: any) {
      error = e.message || 'Failed to create sandbox';
    } finally {
      creating = false;
    }
  }

  function profileProvisionFields(): { profile_id?: string; profile_revision?: string } {
    const profileId = selectedProfile?.profile.id;
    if (!profileId) return {};
    const revision = selectedProfile?.profile.revision ?? selectedProfile?.asset_status?.profile_revision ?? null;
    return {
      profile_id: profileId,
      ...(revision ? { profile_revision: revision } : {}),
    };
  }

  function profileUsable(profile: ProfileListRecord): boolean {
    return profile.asset_status?.usable_for_vm !== false;
  }

  function profileName(profile: ProfileListRecord): string {
    return profile.profile.name || profile.profile.id;
  }

  function profileDescription(profile: ProfileListRecord): string {
    return profile.profile.description || 'A ready-to-use Capsem session profile.';
  }

  function profileBestFor(profile: ProfileListRecord): string {
    return profile.profile.best_for || 'General agent work.';
  }

  function profileRevision(profile: ProfileListRecord): string | null {
    return profile.profile.revision ?? profile.asset_status?.profile_revision ?? null;
  }

  function profileAssetLabel(profile: ProfileListRecord): string {
    if (profileUsable(profile)) return 'Ready';
    if (profile.asset_status?.state === 'missing') return 'Assets missing';
    return 'Unavailable';
  }
</script>

<Modal
  open={vmStore.showCreateModal}
  title="New Session"
  confirmLabel={creating ? 'Creating...' : 'Create'}
  onconfirm={handleSubmit}
  oncancel={close}
  disabled={createDisabled}
>
  <div class="space-y-4 py-2">
    {#if error}
      <div class="p-3 rounded-lg bg-destructive/10 border border-destructive/20 text-destructive text-sm">
        {error}
      </div>
    {/if}

    <div class="space-y-2">
      <span class="text-sm font-medium text-foreground">Profile</span>
      {#if loadingProfiles}
        <div class="bg-card border border-card-line rounded-xl p-4 text-sm text-muted-foreground-1">
          Loading profiles...
        </div>
      {:else if profileError}
        <div class="bg-card border border-card-line rounded-xl p-4 flex items-start gap-x-3">
          <WarningCircle class="shrink-0 text-destructive" size={18} />
          <p class="text-sm text-destructive">{profileError}</p>
        </div>
      {:else if profiles.length === 0}
        <div class="bg-card border border-card-line rounded-xl p-4 text-sm text-muted-foreground-1">
          No profiles installed.
        </div>
      {:else}
        <div class="grid grid-cols-1 gap-2">
          {#each profiles as profile (profile.profile.id)}
            <button
              type="button"
              class="w-full text-left bg-card border rounded-xl p-3 transition-colors hover:bg-layer disabled:cursor-not-allowed disabled:opacity-60"
              class:border-primary={selectedProfileId === profile.profile.id}
              class:border-card-line={selectedProfileId !== profile.profile.id}
              disabled={creating || !profileUsable(profile)}
              onclick={() => selectedProfileId = profile.profile.id}
            >
              <div class="flex gap-3">
                <div class="size-10 shrink-0 rounded-lg bg-primary/10 text-primary flex items-center justify-center">
                  {#if profile.profile.ui === 'coding'}
                    <BracketsAngle size={20} />
                  {:else}
                    <Briefcase size={20} />
                  {/if}
                </div>
                <div class="min-w-0 flex-1">
                  <div class="flex items-center gap-2 flex-wrap">
                    <span class="text-sm font-medium text-foreground">{profileName(profile)}</span>
                    <span
                      class="text-[10px] px-1.5 py-0.5 rounded-full {profileUsable(profile) ? 'bg-primary/10 text-primary' : 'bg-destructive/10 text-destructive'}"
                    >
                      {profileAssetLabel(profile)}
                    </span>
                  </div>
                  <p class="mt-1 text-xs text-muted-foreground-1">{profileDescription(profile)}</p>
                  <p class="mt-1 text-xs text-muted-foreground">{profileBestFor(profile)}</p>
                  {#if profileRevision(profile)}
                    <p class="mt-2 text-[11px] text-muted-foreground-1">{profileRevision(profile)}</p>
                  {/if}
                </div>
              </div>
            </button>
          {/each}
        </div>
      {/if}
    </div>

    <div class="space-y-1.5">
      <label for="sb-name" class="text-sm font-medium text-foreground">Name <span class="text-muted-foreground font-normal">(optional)</span></label>
      <input
        id="sb-name"
        type="text"
        bind:value={name}
        placeholder="Leave empty for a temporary session"
        class="w-full px-3 py-2 rounded-lg bg-background-1 border border-line-2 focus:border-primary focus:ring-2 focus:ring-primary/20 outline-hidden transition-all text-sm text-foreground"
        disabled={creating}
      />
      <p class="text-[11px] text-muted-foreground-1">Named sessions are persistent. Unnamed sessions are ephemeral.</p>
    </div>

    <div class="space-y-2">
      <span class="text-sm font-medium text-foreground">Resources</span>
      <div class="inline-flex rounded-lg border border-line-2 bg-layer p-0.5">
        <button
          type="button"
          class="px-3 py-1.5 text-sm rounded-md transition-colors {resourceMode === 'service' ? 'bg-primary text-primary-foreground' : 'text-muted-foreground-1 hover:text-foreground'}"
          onclick={() => resourceMode = 'service'}
          disabled={creating}
        >
          Service default
        </button>
        <button
          type="button"
          class="px-3 py-1.5 text-sm rounded-md transition-colors {resourceMode === 'custom' ? 'bg-primary text-primary-foreground' : 'text-muted-foreground-1 hover:text-foreground'}"
          onclick={() => resourceMode = 'custom'}
          disabled={creating}
        >
          Override
        </button>
      </div>
      {#if resourceMode === 'custom'}
        <div class="grid grid-cols-2 gap-4">
          <div class="space-y-1.5">
            <label for="sb-ram" class="text-sm font-medium text-foreground">RAM (MB)</label>
            <select
              id="sb-ram"
              bind:value={ramMb}
              class="w-full px-3 py-2 rounded-lg bg-background-1 border border-line-2 focus:border-primary outline-hidden text-sm text-foreground"
              disabled={creating}
            >
              <option value={1024}>1024 MB (1 GB)</option>
              <option value={2048}>2048 MB (2 GB)</option>
              <option value={4096}>4096 MB (4 GB)</option>
              <option value={8192}>8192 MB (8 GB)</option>
            </select>
          </div>

          <div class="space-y-1.5">
            <label for="sb-cpus" class="text-sm font-medium text-foreground">CPUs</label>
            <select
              id="sb-cpus"
              bind:value={cpus}
              class="w-full px-3 py-2 rounded-lg bg-background-1 border border-line-2 focus:border-primary outline-hidden text-sm text-foreground"
              disabled={creating}
            >
              <option value={1}>1 CPU</option>
              <option value={2}>2 CPUs</option>
              <option value={4}>4 CPUs</option>
              <option value={8}>8 CPUs</option>
            </select>
          </div>
        </div>
      {/if}
    </div>
  </div>
</Modal>
