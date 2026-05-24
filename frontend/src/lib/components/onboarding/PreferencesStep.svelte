<script lang="ts">
  import { onMount } from 'svelte';
  import * as api from '../../api';
  import type { ProfileListRecord } from '../../types/gateway';
  import { themeStore, PRELINE_THEMES } from '../../stores/theme.svelte.ts';
  import { THEME_FAMILIES } from '../../terminal/themes';

  // -- Profiles --
  let profiles = $state<ProfileListRecord[]>([]);
  let selectedProfile = $state<string>('');
  let profileLoadError = $state<string | null>(null);

  const defaultCpuCores = 4;
  const defaultRamGb = 8;
  const defaultActiveVms = 8;

  onMount(async () => {
    try {
      const response = await api.listProfiles();
      profiles = response.profiles;
      selectedProfile = response.default_profile ?? '';
    } catch (error) {
      profileLoadError = error instanceof Error ? error.message : String(error);
    }
  });

  function profileId(record: ProfileListRecord): string {
    return record.profile.id;
  }

  function profileRevision(record: ProfileListRecord): string | null {
    return record.profile.revision ?? record.asset_status?.profile_revision ?? null;
  }

  function profileSelectionBlocked(record: ProfileListRecord): boolean {
    return record.asset_status?.usable_for_vm === false;
  }

  async function onProfileChange() {
    if (selectedProfile) {
      try {
        await api.selectProfile(selectedProfile);
        const response = await api.listProfiles();
        profiles = response.profiles;
        selectedProfile = response.default_profile ?? selectedProfile;
        profileLoadError = null;
      } catch (error) {
        profileLoadError = error instanceof Error ? error.message : String(error);
      }
    }
  }

</script>

<div class="space-y-5">
  <div>
    <h2 class="text-xl font-medium text-foreground">Preferences</h2>
    <p class="mt-1 text-sm text-muted-foreground-1">
      Customize your experience. All settings can be changed later.
    </p>
  </div>

  <!-- Profile (compact) -->
  <div class="bg-card border border-card-line rounded-xl p-4">
    <div class="flex items-center justify-between">
      <div>
        <span class="text-sm font-medium text-foreground">Profile</span>
        <p class="text-xs text-muted-foreground mt-0.5">Controls VM assets, tools, MCP, and security rules.</p>
      </div>
      <select
        class="py-1.5 px-3 text-sm border border-line-2 rounded-lg bg-layer text-foreground focus:border-primary focus:ring-1 focus:ring-primary outline-none"
        bind:value={selectedProfile}
        onchange={onProfileChange}
        disabled={profiles.length === 0}
      >
        <option value="" disabled>{profiles.length === 0 ? 'No profiles' : 'Select profile'}</option>
        {#each profiles as profile}
          <option value={profileId(profile)} disabled={profileSelectionBlocked(profile)}>
            {profile.profile.name || profileId(profile)}{profileRevision(profile) ? `@${profileRevision(profile)}` : ''}
          </option>
        {/each}
      </select>
    </div>
    {#if profileLoadError}
      <p class="mt-2 text-xs text-destructive">{profileLoadError}</p>
    {/if}
  </div>

  <!-- Appearance -->
  <div class="bg-card border border-card-line rounded-xl p-4 space-y-4">
    <h3 class="text-sm font-medium text-foreground">Appearance</h3>

    <!-- Dark mode -->
    <div class="flex items-center justify-between">
      <span class="text-sm text-muted-foreground-1">Dark mode</span>
      <div class="flex gap-1">
        {#each ['auto', 'light', 'dark'] as mode}
          <button
            type="button"
            class="py-1 px-2.5 text-xs rounded-md transition-colors"
            class:bg-primary={themeStore.modePref === mode}
            class:text-primary-foreground={themeStore.modePref === mode}
            class:bg-layer={themeStore.modePref !== mode}
            class:text-muted-foreground-1={themeStore.modePref !== mode}
            onclick={() => themeStore.setMode(mode as 'auto' | 'light' | 'dark')}
          >
            {mode.charAt(0).toUpperCase() + mode.slice(1)}
          </button>
        {/each}
      </div>
    </div>

    <!-- UI theme -->
    <div class="flex items-center justify-between">
      <span class="text-sm text-muted-foreground-1">Accent theme</span>
      <div class="flex gap-1.5">
        {#each PRELINE_THEMES as t}
          <button
            type="button"
            class="size-5 rounded-full border-2 transition-colors"
            class:border-foreground={themeStore.prelineTheme === t.value}
            class:border-transparent={themeStore.prelineTheme !== t.value}
            style="background-color: {t.color}"
            title={t.label}
            onclick={() => themeStore.setPrelineTheme(t.value)}
          ></button>
        {/each}
      </div>
    </div>

    <!-- Terminal theme -->
    <div class="flex items-center justify-between">
      <span class="text-sm text-muted-foreground-1">Terminal theme</span>
      <select
        class="py-1.5 px-3 text-sm border border-line-2 rounded-lg bg-layer text-foreground focus:border-primary focus:ring-1 focus:ring-primary outline-none"
        value={themeStore.terminalTheme}
        onchange={(e) => themeStore.setTerminalTheme((e.target as HTMLSelectElement).value)}
      >
        {#each THEME_FAMILIES as family}
          <option value={family.name}>{family.label}</option>
        {/each}
      </select>
    </div>
  </div>

  <!-- VM defaults -->
  <div class="bg-card border border-card-line rounded-xl p-4 space-y-4">
    <h3 class="text-sm font-medium text-foreground">VM Defaults</h3>

    <div class="flex items-center justify-between">
      <span class="text-sm text-muted-foreground-1">CPU cores</span>
      <span class="text-sm font-medium text-foreground">{defaultCpuCores}</span>
    </div>

    <div class="flex items-center justify-between">
      <span class="text-sm text-muted-foreground-1">RAM</span>
      <span class="text-sm font-medium text-foreground">{defaultRamGb} GB</span>
    </div>

    <div class="flex items-center justify-between">
      <span class="text-sm text-muted-foreground-1">Active VMs</span>
      <span class="text-sm font-medium text-foreground">{defaultActiveVms}</span>
    </div>
  </div>
</div>
