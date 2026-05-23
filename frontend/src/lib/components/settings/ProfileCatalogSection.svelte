<script lang="ts">
  import { onMount } from 'svelte';
  import { getProfileCatalog } from '../../api';
  import type {
    ProfileCatalogProfile,
    ProfileCatalogResponse,
    ProfileRevisionStatus,
  } from '../../types/gateway';
  import ArrowClockwise from 'phosphor-svelte/lib/ArrowClockwise';
  import WarningCircle from 'phosphor-svelte/lib/WarningCircle';

  let loading = $state(false);
  let error = $state<string | null>(null);
  let catalog = $state<ProfileCatalogResponse | null>(null);

  async function refreshCatalog() {
    loading = true;
    error = null;
    try {
      catalog = await getProfileCatalog();
    } catch (err) {
      error = String(err instanceof Error ? err.message : err);
      catalog = null;
    } finally {
      loading = false;
    }
  }

  function statusClasses(status: ProfileRevisionStatus): string {
    if (status === 'active') return 'bg-primary/10 text-primary';
    if (status === 'deprecated') return 'bg-warning/10 text-warning';
    return 'bg-destructive/10 text-destructive';
  }

  function profileState(profile: ProfileCatalogProfile): string {
    if (!profile.installed_revision) return 'not installed';
    if (profile.current_revision && profile.current_revision !== profile.installed_revision) {
      return 'update available';
    }
    return 'current';
  }

  function profileStateClasses(profile: ProfileCatalogProfile): string {
    const state = profileState(profile);
    if (state === 'current') return 'bg-primary/10 text-primary';
    if (state === 'update available') return 'bg-warning/10 text-warning';
    return 'bg-muted text-muted-foreground-1';
  }

  onMount(() => {
    refreshCatalog();
  });
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between gap-x-4">
    <div>
      <h2 class="text-xl font-medium text-foreground">Profiles</h2>
      <p class="text-sm text-muted-foreground-1 mt-0.5">Installed catalog revisions and profile lifecycle status.</p>
    </div>
    <button
      type="button"
      class="p-2 rounded-lg border border-line-2 bg-layer text-foreground hover:bg-layer-hover transition-colors disabled:opacity-60"
      title="Refresh profiles"
      aria-label="Refresh profiles"
      disabled={loading}
      onclick={refreshCatalog}
    >
      <ArrowClockwise size={16} />
    </button>
  </div>

  {#if loading && !catalog}
    <div class="bg-card border border-card-line rounded-xl p-6 text-center">
      <p class="text-sm text-muted-foreground-1">Loading profiles...</p>
    </div>
  {:else if error}
    <div class="bg-card border border-card-line rounded-xl p-4 flex items-start gap-x-3">
      <WarningCircle class="shrink-0 text-destructive" size={18} />
      <p class="text-sm text-destructive">{error}</p>
    </div>
  {:else if catalog && (!catalog.manifest_present || catalog.profiles.length === 0)}
    <div class="bg-card border border-card-line rounded-xl p-6 text-center">
      <p class="text-sm text-muted-foreground-1">No profile catalog installed.</p>
    </div>
  {:else if catalog}
    <div class="space-y-3">
      {#each catalog.profiles as profile (profile.profile_id)}
        <article class="bg-card border border-card-line rounded-xl p-4">
          <div class="flex items-start justify-between gap-x-4">
            <div class="min-w-0">
              <div class="flex items-center gap-x-2 flex-wrap">
                <h3 class="text-sm font-mono text-foreground">{profile.profile_id}</h3>
                <span class="text-[10px] px-1.5 py-0.5 rounded-full {profileStateClasses(profile)}">
                  {profileState(profile)}
                </span>
              </div>
              <dl class="grid grid-cols-1 sm:grid-cols-2 gap-x-6 gap-y-1 mt-3 text-xs">
                <dt class="text-muted-foreground-1">Installed revision</dt>
                <dd class="text-foreground font-mono break-all">{profile.installed_revision ?? 'none'}</dd>
                <dt class="text-muted-foreground-1">Current revision</dt>
                <dd class="text-foreground font-mono break-all">{profile.current_revision ?? 'none'}</dd>
              </dl>
            </div>
          </div>

          <div class="mt-4 border-t border-card-divider pt-3">
            <div class="flex items-center justify-between gap-x-3 mb-2">
              <p class="text-xs font-semibold text-foreground uppercase tracking-wider">Revisions</p>
              <span class="text-xs text-muted-foreground-1">{profile.revisions.length} revision{profile.revisions.length === 1 ? '' : 's'}</span>
            </div>
            <div class="space-y-2">
              {#each profile.revisions as revision (revision.revision)}
                <div class="flex items-center justify-between gap-x-3 rounded-lg border border-line-2 bg-layer px-3 py-2">
                  <div class="min-w-0">
                    <p class="text-xs font-mono text-foreground break-all">{revision.revision}</p>
                    {#if revision.profile_hash}
                      <p class="text-[11px] font-mono text-muted-foreground-1 break-all mt-0.5">{revision.profile_hash}</p>
                    {/if}
                  </div>
                  <div class="flex items-center gap-x-1.5 shrink-0">
                    <span class="text-[10px] px-1.5 py-0.5 rounded-full {statusClasses(revision.status)}">{revision.status}</span>
                    {#if revision.current}
                      <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground-1">current</span>
                    {/if}
                    {#if revision.installed}
                      <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground-1">installed</span>
                    {/if}
                  </div>
                </div>
              {/each}
            </div>
          </div>
        </article>
      {/each}
    </div>
  {/if}
</div>
