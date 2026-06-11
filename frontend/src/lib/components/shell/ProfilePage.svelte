<script lang="ts">
  import { onMount } from 'svelte';
  import {
    listProfiles,
    getProfileInfo,
    getAssetsStatus,
    listEnforcementRules,
    listDetectionRules,
    type EnforcementRuleInfo,
    type ProfileInfoResponse,
    type ProfileSummary,
    type SecurityRuleAction,
    type SecurityRuleDetectionLevel,
  } from '../../api';
  import McpSection from '../settings/McpSection.svelte';
  import PluginSection from '../settings/PluginSection.svelte';
  import type { AssetEntry, AssetStatusResponse } from '../../types/assets';
  import Shield from 'phosphor-svelte/lib/Shield';
  import Plugs from 'phosphor-svelte/lib/Plugs';
  import HardDrives from 'phosphor-svelte/lib/HardDrives';
  import IdentificationCard from 'phosphor-svelte/lib/IdentificationCard';
  import CheckCircle from 'phosphor-svelte/lib/CheckCircle';
  import CircleNotch from 'phosphor-svelte/lib/CircleNotch';
  import HandPalm from 'phosphor-svelte/lib/HandPalm';
  import PencilSimple from 'phosphor-svelte/lib/PencilSimple';
  import Prohibit from 'phosphor-svelte/lib/Prohibit';
  import WarningCircle from 'phosphor-svelte/lib/WarningCircle';

  type Section = 'overview' | 'enforcement' | 'detection' | 'plugins' | 'mcp' | 'assets';
  let activeSection = $state<Section>('overview');
  let profiles = $state<ProfileSummary[]>([]);
  let profileId = $state('');
  let loading = $state(true);
  let error = $state<string | null>(null);
  let profile = $state<ProfileInfoResponse | null>(null);
  let assetsInfo = $state<AssetStatusResponse | null>(null);
  let enforcementRules = $state<EnforcementRuleInfo[]>([]);
  let detectionRules = $state<EnforcementRuleInfo[]>([]);

  const navItems: { key: Section; label: string; icon: typeof Shield }[] = [
    { key: 'overview', label: 'Overview', icon: IdentificationCard },
    { key: 'enforcement', label: 'Enforcement', icon: Shield },
    { key: 'detection', label: 'Detection', icon: Shield },
    { key: 'plugins', label: 'Plugins', icon: Plugs },
    { key: 'mcp', label: 'MCP', icon: Plugs },
    { key: 'assets', label: 'Assets', icon: HardDrives },
  ];

  const ACTION_META: Record<SecurityRuleAction, { label: string; icon: typeof CheckCircle; tone: string }> = {
    allow: {
      label: 'Allow',
      icon: CheckCircle,
      tone: 'text-primary border-primary/30 bg-primary/10',
    },
    ask: {
      label: 'Ask',
      icon: HandPalm,
      tone: 'text-warning border-warning/30 bg-warning/10',
    },
    block: {
      label: 'Block',
      icon: Prohibit,
      tone: 'text-destructive-foreground border-destructive/30 bg-destructive/10',
    },
    preprocess: {
      label: 'Preprocess',
      icon: PencilSimple,
      tone: 'text-primary border-primary/30 bg-primary/10',
    },
    rewrite: {
      label: 'Rewrite',
      icon: PencilSimple,
      tone: 'text-primary border-primary/30 bg-primary/10',
    },
    postprocess: {
      label: 'Postprocess',
      icon: PencilSimple,
      tone: 'text-primary border-primary/30 bg-primary/10',
    },
  };

  const DETECTION_META: Record<SecurityRuleDetectionLevel | 'none', { label: string; tone: string }> = {
    none: {
      label: 'None',
      tone: 'text-muted-foreground-2 border-line-2 bg-muted/40',
    },
    informational: {
      label: 'Informational',
      tone: 'text-muted-foreground-1 border-line-2 bg-muted/40',
    },
    low: {
      label: 'Low',
      tone: 'text-primary border-primary/30 bg-primary/10',
    },
    medium: {
      label: 'Medium',
      tone: 'text-warning border-warning/30 bg-warning/10',
    },
    high: {
      label: 'High',
      tone: 'text-destructive-foreground border-destructive/30 bg-destructive/10',
    },
    critical: {
      label: 'Critical',
      tone: 'text-destructive-foreground border-destructive/40 bg-destructive/15',
    },
  };

  function actionMeta(action: SecurityRuleAction) {
    return ACTION_META[action];
  }

  function detectionMeta(level: SecurityRuleDetectionLevel | undefined) {
    return DETECTION_META[level ?? 'none'];
  }

  onMount(() => {
    void load();
  });

  async function load() {
    loading = true;
    error = null;
    try {
      const profileList = await listProfiles();
      profiles = profileList.profiles;
      const activeProfileId = profileId || profiles[0]?.id;
      if (!activeProfileId) throw new Error('No profiles available');
      profileId = activeProfileId;
      await loadProfile(activeProfileId);
    } catch (err) {
      error = String(err instanceof Error ? err.message : err);
    } finally {
      loading = false;
    }
  }

  async function loadProfile(activeProfileId: string) {
    error = null;
    try {
      const [profileResult, assetsResult, enforcementResult, detectionResult] = await Promise.all([
        getProfileInfo(activeProfileId),
        getAssetsStatus(activeProfileId),
        listEnforcementRules(activeProfileId),
        listDetectionRules(activeProfileId),
      ]);
      profile = profileResult;
      assetsInfo = assetsResult;
      enforcementRules = enforcementResult.rules;
      detectionRules = detectionResult.rules;
    } catch (err) {
      error = String(err instanceof Error ? err.message : err);
    } finally {
      loading = false;
    }
  }

  async function selectProfile(nextProfileId: string) {
    if (!nextProfileId || nextProfileId === profileId) return;
    profileId = nextProfileId;
    loading = true;
    try {
      await loadProfile(nextProfileId);
    } finally {
      loading = false;
    }
  }

  function sourceLabel(rule: EnforcementRuleInfo): string {
    return `${rule.source}${rule.default_rule ? ' default' : ''}`;
  }

  function assetStatusLabel(asset: AssetEntry): string {
    if (asset.status === 'present') return 'Verified';
    if (asset.status === 'downloading') return 'Downloading';
    if (asset.status === 'missing') return 'Missing';
    return 'Invalid';
  }

  function assetStatusClass(asset: AssetEntry): string {
    if (asset.status === 'present') return 'text-primary bg-primary/10';
    if (asset.status === 'downloading') return 'text-muted-foreground-1 bg-muted';
    return 'text-destructive-foreground bg-destructive/10';
  }

  function assetTitle(asset: AssetEntry): string {
    return asset.kind ?? asset.name;
  }

  function formatBytes(bytes?: number | null): string {
    if (!bytes || bytes <= 0) return '--';
    const units = ['B', 'KB', 'MB', 'GB'];
    let value = bytes;
    let unit = 0;
    while (value >= 1024 && unit < units.length - 1) {
      value /= 1024;
      unit += 1;
    }
    return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
  }
</script>

<div class="flex h-full">
  <aside class="w-56 shrink-0 border-e border-line-2 bg-background overflow-y-auto py-4">
    <h1 class="text-xl font-bold text-foreground px-5 mb-4">Profile</h1>
    {#if profiles.length > 0}
      <div class="px-3 mb-4">
        <label for="profile-select" class="text-xs font-semibold text-muted-foreground-1 uppercase tracking-wider block mb-1">Profile</label>
        <select
          id="profile-select"
          class="w-full py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
          value={profileId}
          onchange={(event) => selectProfile((event.target as HTMLSelectElement).value)}
        >
          {#each profiles as option (option.id)}
            <option value={option.id}>{option.name}</option>
          {/each}
        </select>
      </div>
    {/if}
    <nav class="space-y-0.5 px-3">
      {#each navItems as item (item.key)}
        <button
          type="button"
          class="w-full flex items-center gap-x-3 py-2 px-3 text-sm rounded-lg transition-colors
            {activeSection === item.key
              ? 'bg-muted text-foreground font-medium'
              : 'text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover'}"
          onclick={() => activeSection = item.key}
        >
          <item.icon size={18} />
          {item.label}
        </button>
      {/each}
    </nav>
  </aside>

  <main class="flex-1 overflow-y-auto relative">
    {#if loading}
      <div class="flex items-center justify-center h-full">
        <div class="animate-spin size-6 border-2 border-primary border-t-transparent rounded-full"></div>
      </div>
    {:else if error}
      <div class="flex flex-col items-center justify-center h-full gap-y-4">
        <p class="text-sm text-destructive-foreground">{error}</p>
        <button
          type="button"
          class="py-2 px-4 text-sm font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover transition-colors"
          onclick={load}
        >
          Retry
        </button>
      </div>
    {:else}
      <div class="py-6 px-8">
        {#if activeSection === 'overview' && profile}
          <h2 class="text-xl font-medium text-foreground mb-6">{profile.profile.name}</h2>
          <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
            <div class="grid grid-cols-[12rem_minmax(0,1fr)] gap-x-4 p-4">
              <p class="text-sm text-muted-foreground-1">ID</p>
              <p class="text-sm font-mono text-foreground">{profile.profile.id}</p>
            </div>
            <div class="grid grid-cols-[12rem_minmax(0,1fr)] gap-x-4 p-4">
              <p class="text-sm text-muted-foreground-1">Description</p>
              <p class="text-sm text-foreground">{profile.profile.description}</p>
            </div>
            <div class="grid grid-cols-[12rem_minmax(0,1fr)] gap-x-4 p-4">
              <p class="text-sm text-muted-foreground-1">Source</p>
              <p class="text-sm text-foreground">{profile.profile.source}</p>
            </div>
            <div class="grid grid-cols-4 gap-4 p-4">
              <div>
                <p class="text-xs text-muted-foreground-1">Rules</p>
                <p class="text-lg font-semibold text-foreground">{profile.profile.rule_count}</p>
              </div>
              <div>
                <p class="text-xs text-muted-foreground-1">Defaults</p>
                <p class="text-lg font-semibold text-foreground">{profile.profile.default_rule_count}</p>
              </div>
              <div>
                <p class="text-xs text-muted-foreground-1">Plugins</p>
                <p class="text-lg font-semibold text-foreground">{profile.profile.plugin_count}</p>
              </div>
              <div>
                <p class="text-xs text-muted-foreground-1">MCP</p>
                <p class="text-lg font-semibold text-foreground">{profile.profile.mcp_server_count}</p>
              </div>
            </div>
          </div>
        {:else if activeSection === 'enforcement'}
          <h2 class="text-xl font-medium text-foreground mb-6">Enforcement</h2>
          <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
            {#each enforcementRules as rule (rule.rule_id)}
              {@const meta = actionMeta(rule.action)}
              <div class="p-4 {rule.enabled ? '' : 'bg-muted/20 opacity-70'}">
                <div class="flex items-start justify-between gap-x-3">
                  <div class="min-w-0">
                    <div class="flex items-center gap-x-2">
                      <p class="text-sm font-medium text-foreground truncate">{rule.name}</p>
                      {#if !rule.enabled}
                        <span class="text-[11px] uppercase tracking-wide text-muted-foreground-2">Disabled</span>
                      {/if}
                    </div>
                    {#if rule.reason}
                      <p class="text-xs text-muted-foreground-1 mt-0.5 line-clamp-2">{rule.reason}</p>
                    {/if}
                  </div>
                  <span class={`inline-flex items-center gap-x-1 rounded-full border px-2 py-0.5 text-xs font-medium shrink-0 ${meta.tone}`}>
                    <meta.icon size={12} weight="fill" />
                    {meta.label}
                  </span>
                </div>
                <p class="text-[11px] text-muted-foreground-2 mt-2 font-mono truncate">{rule.rule_id}</p>
                <p class="text-[11px] text-muted-foreground-2 mt-1">{sourceLabel(rule)} · priority {rule.priority}</p>
              </div>
            {/each}
          </div>
        {:else if activeSection === 'detection'}
          <h2 class="text-xl font-medium text-foreground mb-6">Detection</h2>
          <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
            {#each detectionRules as rule (rule.rule_id)}
              {@const meta = detectionMeta(rule.detection_level)}
              <div class="p-4 {rule.enabled ? '' : 'bg-muted/20 opacity-70'}">
                <div class="flex items-start justify-between gap-x-3">
                  <div class="min-w-0">
                    <div class="flex items-center gap-x-2">
                      <p class="text-sm font-medium text-foreground truncate">{rule.name}</p>
                      {#if !rule.enabled}
                        <span class="text-[11px] uppercase tracking-wide text-muted-foreground-2">Disabled</span>
                      {/if}
                    </div>
                    {#if rule.reason}
                      <p class="text-xs text-muted-foreground-1 mt-0.5 line-clamp-2">{rule.reason}</p>
                    {/if}
                  </div>
                  <span class={`inline-flex items-center gap-x-1 rounded-full border px-2 py-0.5 text-xs font-medium shrink-0 ${meta.tone}`}>
                    <WarningCircle size={12} weight="fill" />
                    {meta.label}
                  </span>
                </div>
                <p class="text-[11px] text-muted-foreground-2 mt-2 font-mono truncate">{rule.rule_id}</p>
                <p class="text-[11px] text-muted-foreground-2 mt-1">{sourceLabel(rule)} · priority {rule.priority}</p>
              </div>
            {/each}
          </div>
        {:else if activeSection === 'plugins'}
          <PluginSection {profileId} />
        {:else if activeSection === 'mcp'}
          <McpSection {profileId} />
        {:else if activeSection === 'assets'}
          <h2 class="text-xl font-medium text-foreground mb-6">Assets</h2>
          {#if assetsInfo}
            <div class="space-y-4">
              <div class="bg-card border border-card-line rounded-xl p-4">
                <div class="flex items-center justify-between gap-x-4">
                  <div>
                    <p class="text-sm font-medium text-foreground">Profile asset readiness</p>
                    <p class="text-xs text-muted-foreground-1 mt-1">
                      {assetsInfo.ready ? 'All required assets and profile files are verified.' : 'One or more required assets or profile files need attention.'}
                    </p>
                  </div>
                  <span class="inline-flex items-center gap-x-1.5 rounded-full px-2.5 py-1 text-xs font-medium {assetsInfo.ready ? 'bg-primary/10 text-primary' : 'bg-destructive/10 text-destructive-foreground'}">
                    {#if assetsInfo.downloading}
                      <CircleNotch size={14} class="animate-spin" />
                      Downloading
                    {:else if assetsInfo.ready}
                      <CheckCircle size={14} />
                      Ready
                    {:else}
                      <WarningCircle size={14} />
                      Attention
                    {/if}
                  </span>
                </div>
                {#if assetsInfo.manifest}
                  <div class="mt-4 grid gap-3 text-xs sm:grid-cols-2">
                    <div>
                      <p class="text-muted-foreground-1">Manifest</p>
                      <p class="font-mono text-foreground truncate">{assetsInfo.manifest.origin_source ?? assetsInfo.manifest.origin}</p>
                    </div>
                    <div>
                      <p class="text-muted-foreground-1">Hash</p>
                      <p class="font-mono text-foreground truncate">{assetsInfo.manifest.blake3 ?? '--'}</p>
                    </div>
                  </div>
                {/if}
              </div>

              <section>
                <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2">VM assets</h3>
                <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
                  {#each assetsInfo.assets as asset (`${asset.arch ?? ''}:${asset.kind ?? asset.name}`)}
                    <div class="p-4 flex items-start gap-x-3">
                      {#if asset.status === 'present'}
                        <CheckCircle size={18} class="text-primary shrink-0 mt-0.5" />
                      {:else if asset.status === 'downloading'}
                        <CircleNotch size={18} class="text-muted-foreground-1 animate-spin shrink-0 mt-0.5" />
                      {:else}
                        <WarningCircle size={18} class="text-destructive-foreground shrink-0 mt-0.5" />
                      {/if}
                      <div class="min-w-0 flex-1">
                        <div class="flex items-center justify-between gap-x-3">
                          <p class="text-sm font-medium text-foreground truncate">{assetTitle(asset)}</p>
                          <span class="rounded-full px-2 py-0.5 text-xs shrink-0 {assetStatusClass(asset)}">{assetStatusLabel(asset)}</span>
                        </div>
                        <p class="text-xs text-muted-foreground-1 font-mono truncate mt-1">{asset.path ?? asset.name}</p>
                        <p class="text-[11px] text-muted-foreground-2 mt-1">
                          Expected {formatBytes(asset.expected_size)} · actual {formatBytes(asset.actual_size)}
                        </p>
                      </div>
                    </div>
                  {/each}
                </div>
              </section>

              {#if assetsInfo.files && assetsInfo.files.length > 0}
                <section>
                  <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2">Profile files</h3>
                  <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
                    {#each assetsInfo.files as file (file.kind ?? file.path ?? file.name)}
                      <div class="p-4 flex items-start gap-x-3">
                        {#if file.status === 'present'}
                          <CheckCircle size={18} class="text-primary shrink-0 mt-0.5" />
                        {:else}
                          <WarningCircle size={18} class="text-destructive-foreground shrink-0 mt-0.5" />
                        {/if}
                        <div class="min-w-0 flex-1">
                          <div class="flex items-center justify-between gap-x-3">
                            <p class="text-sm font-medium text-foreground truncate">{assetTitle(file)}</p>
                            <span class="rounded-full px-2 py-0.5 text-xs shrink-0 {assetStatusClass(file)}">{assetStatusLabel(file)}</span>
                          </div>
                          <p class="text-xs text-muted-foreground-1 font-mono truncate mt-1">{file.path ?? file.name}</p>
                        </div>
                      </div>
                    {/each}
                  </div>
                </section>
              {/if}
            </div>
          {/if}
        {/if}
      </div>
    {/if}
  </main>
</div>
