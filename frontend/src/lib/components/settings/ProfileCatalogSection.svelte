<script lang="ts">
  import { onMount } from 'svelte';
  import { getProfileCatalog, listProfiles, selectProfile } from '../../api';
  import type {
    ProfileCatalogResponse,
    ProfileListRecord,
  } from '../../types/gateway';
  import ArrowClockwise from 'phosphor-svelte/lib/ArrowClockwise';
  import BracketsAngle from 'phosphor-svelte/lib/BracketsAngle';
  import Briefcase from 'phosphor-svelte/lib/Briefcase';
  import WarningCircle from 'phosphor-svelte/lib/WarningCircle';

  let loading = $state(false);
  let selectingProfileId = $state<string | null>(null);
  let error = $state<string | null>(null);
  let statusMessage = $state<string | null>(null);
  let defaultProfile = $state<string | null>(null);
  let profiles = $state<ProfileListRecord[]>([]);
  let catalog = $state<ProfileCatalogResponse | null>(null);

  async function refreshProfiles() {
    loading = true;
    error = null;
    statusMessage = null;
    try {
      const response = await listProfiles();
      profiles = response.profiles;
      defaultProfile = response.default_profile ?? null;
      try {
        catalog = await getProfileCatalog();
      } catch {
        catalog = null;
      }
    } catch (err) {
      error = String(err instanceof Error ? err.message : err);
      profiles = [];
      catalog = null;
    } finally {
      loading = false;
    }
  }

  async function selectDefaultProfile(record: ProfileListRecord) {
    if (isSelected(record) || profileSelectionBlocked(record)) return;
    selectingProfileId = profileId(record);
    error = null;
    statusMessage = null;
    try {
      await selectProfile(profileId(record));
      await refreshProfiles();
      statusMessage = `${profileName(record)} selected.`;
    } catch (err) {
      error = String(err instanceof Error ? err.message : err);
    } finally {
      selectingProfileId = null;
    }
  }

  function profileId(record: ProfileListRecord): string {
    return record.profile.id;
  }

  function profileName(record: ProfileListRecord): string {
    return record.profile.name || profileId(record);
  }

  function profileDescription(record: ProfileListRecord): string {
    return record.profile.description || 'A ready-to-use Capsem session profile.';
  }

  function profileBestFor(record: ProfileListRecord): string {
    return record.profile.best_for || 'General agent work.';
  }

  function profileRevision(record: ProfileListRecord): string | null {
    return record.profile.revision ?? record.asset_status?.profile_revision ?? null;
  }

  function isSelected(record: ProfileListRecord): boolean {
    return defaultProfile === profileId(record);
  }

  function profileSelectionBlocked(record: ProfileListRecord): boolean {
    return record.ui === false || record.web === false || record.asset_status?.usable_for_vm === false;
  }

  function assetStateLabel(record: ProfileListRecord): string {
    if (!record.asset_status) return 'asset status unknown';
    if (record.asset_status.usable_for_vm) return 'ready';
    if (record.asset_status.state === 'missing') return 'assets missing';
    return 'unavailable';
  }

  function assetStateClasses(record: ProfileListRecord): string {
    if (!record.asset_status) return 'bg-muted text-muted-foreground-1';
    if (record.asset_status.usable_for_vm) return 'bg-primary/10 text-primary';
    if (record.asset_status.state === 'missing') return 'bg-warning/10 text-warning';
    return 'bg-destructive/10 text-destructive';
  }

  function sourceLabel(record: ProfileListRecord): string {
    if (record.source === 'base') return 'Built in';
    if (record.source === 'corp') return 'Corp managed';
    if (record.source === 'user') return 'User';
    return record.source;
  }

  function profileSelectionBlockedReason(record: ProfileListRecord): string {
    if (profileSelectionBlocked(record)) return 'Profiles with missing or invalid assets cannot be used for sessions';
    return 'Select default profile';
  }

  onMount(() => {
    refreshProfiles();
  });
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between gap-x-4">
    <div>
      <h2 class="text-xl font-medium text-foreground">Profiles</h2>
      <p class="text-sm text-muted-foreground-1 mt-0.5">Choose the default session profile.</p>
    </div>
    <button
      type="button"
      class="p-2 rounded-lg border border-line-2 bg-layer text-foreground hover:bg-layer-hover transition-colors disabled:opacity-60"
      title="Refresh profiles"
      aria-label="Refresh profiles"
      disabled={loading}
      onclick={refreshProfiles}
    >
      <ArrowClockwise size={16} />
    </button>
  </div>

  {#if loading && profiles.length === 0}
    <div class="bg-card border border-card-line rounded-xl p-6 text-center">
      <p class="text-sm text-muted-foreground-1">Loading profiles...</p>
    </div>
  {:else if error && profiles.length === 0}
    <div class="bg-card border border-card-line rounded-xl p-4 flex items-start gap-x-3">
      <WarningCircle class="shrink-0 text-destructive" size={18} />
      <p class="text-sm text-destructive">{error}</p>
    </div>
  {:else if profiles.length === 0}
    <div class="bg-card border border-card-line rounded-xl p-6 text-center">
      <p class="text-sm text-muted-foreground-1">No profiles installed.</p>
    </div>
  {:else}
    <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
      {#each profiles as profile (profileId(profile))}
        <article
          class="bg-card border rounded-xl p-4"
          class:border-primary={isSelected(profile)}
          class:border-card-line={!isSelected(profile)}
        >
          <div class="flex gap-3">
            <div class="size-11 shrink-0 rounded-lg bg-primary/10 text-primary flex items-center justify-center">
              {#if profile.profile.ui === 'coding'}
                <BracketsAngle size={21} />
              {:else}
                <Briefcase size={21} />
              {/if}
            </div>

            <div class="min-w-0 flex-1">
              <div class="flex items-center gap-2 flex-wrap">
                <h3 class="text-sm font-medium text-foreground">{profileName(profile)}</h3>
                {#if isSelected(profile)}
                  <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-primary/10 text-primary font-medium">Default</span>
                {/if}
                <span class="text-[10px] px-1.5 py-0.5 rounded-full {assetStateClasses(profile)}">
                  {assetStateLabel(profile)}
                </span>
              </div>
              <p class="mt-1 text-xs text-muted-foreground-1">{profileDescription(profile)}</p>
              <p class="mt-1 text-xs text-muted-foreground">{profileBestFor(profile)}</p>

              <div class="mt-3 flex items-center gap-2 text-[11px] text-muted-foreground-1">
                <span>{sourceLabel(profile)}</span>
                {#if profileRevision(profile)}
                  <span aria-hidden="true">/</span>
                  <span>{profileRevision(profile)}</span>
                {/if}
              </div>
            </div>
          </div>

          <div class="mt-4 flex justify-end">
            <button
              type="button"
              class="py-2 px-3 text-xs font-medium rounded-lg border border-line-2 bg-layer text-foreground hover:bg-layer-hover transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              disabled={isSelected(profile) || profileSelectionBlocked(profile) || selectingProfileId !== null}
              title={profileSelectionBlockedReason(profile)}
              onclick={() => selectDefaultProfile(profile)}
            >
              {#if selectingProfileId === profileId(profile)}
                Selecting...
              {:else if isSelected(profile)}
                Selected
              {:else}
                Select
              {/if}
            </button>
          </div>
        </article>
      {/each}
    </div>

    {#if catalog?.manifest_present}
      <p class="text-xs text-muted-foreground-1">
        Signed catalog connected. Profile revision details are available to administrators.
      </p>
    {/if}

    {#if error}
      <p class="text-xs text-destructive">{error}</p>
    {:else if statusMessage}
      <p class="text-xs text-primary">{statusMessage}</p>
    {/if}
  {/if}
</div>
