<script lang="ts">
  import { onMount } from 'svelte';
  import {
    getProfileInfo,
    getProfileAssetsInfo,
    listEnforcementRules,
    listDetectionRules,
    type EnforcementRuleInfo,
    type ProfileInfoResponse,
  } from '../../api';
  import McpSection from '../settings/McpSection.svelte';
  import PluginSection from '../settings/PluginSection.svelte';
  import Shield from 'phosphor-svelte/lib/Shield';
  import Plugs from 'phosphor-svelte/lib/Plugs';
  import HardDrives from 'phosphor-svelte/lib/HardDrives';
  import IdentificationCard from 'phosphor-svelte/lib/IdentificationCard';

  const PROFILE_ID = 'code';

  type Section = 'overview' | 'policy' | 'plugins' | 'mcp' | 'assets';
  let activeSection = $state<Section>('overview');
  let loading = $state(true);
  let error = $state<string | null>(null);
  let profile = $state<ProfileInfoResponse | null>(null);
  let assetsInfo = $state<unknown>(null);
  let enforcementRules = $state<EnforcementRuleInfo[]>([]);
  let detectionRules = $state<EnforcementRuleInfo[]>([]);

  const navItems: { key: Section; label: string; icon: typeof Shield }[] = [
    { key: 'overview', label: 'Overview', icon: IdentificationCard },
    { key: 'policy', label: 'Policy', icon: Shield },
    { key: 'plugins', label: 'Plugins', icon: Plugs },
    { key: 'mcp', label: 'MCP', icon: Plugs },
    { key: 'assets', label: 'Assets', icon: HardDrives },
  ];

  onMount(() => {
    void load();
  });

  async function load() {
    loading = true;
    error = null;
    try {
      const [profileResult, assetsResult, enforcementResult, detectionResult] = await Promise.all([
        getProfileInfo(PROFILE_ID),
        getProfileAssetsInfo(PROFILE_ID),
        listEnforcementRules(PROFILE_ID),
        listDetectionRules(PROFILE_ID),
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

  function sourceLabel(rule: EnforcementRuleInfo): string {
    return `${rule.source}${rule.default_rule ? ' default' : ''}`;
  }
</script>

<div class="flex h-full">
  <aside class="w-56 shrink-0 border-e border-line-2 bg-background overflow-y-auto py-4">
    <h1 class="text-xl font-bold text-foreground px-5 mb-4">Profile</h1>
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
        {:else if activeSection === 'policy'}
          <h2 class="text-xl font-medium text-foreground mb-6">Policy</h2>
          <div class="grid grid-cols-2 gap-6">
            <section>
              <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2">Enforcement</h3>
              <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
                {#each enforcementRules as rule (rule.rule_id)}
                  <div class="p-4">
                    <div class="flex items-start justify-between gap-x-3">
                      <div class="min-w-0">
                        <p class="text-sm font-medium text-foreground truncate">{rule.name}</p>
                        {#if rule.reason}
                          <p class="text-xs text-muted-foreground-1 mt-0.5 line-clamp-2">{rule.reason}</p>
                        {/if}
                      </div>
                      <span class="text-xs px-2 py-0.5 rounded-full bg-muted text-muted-foreground-1 shrink-0">{rule.action}</span>
                    </div>
                    <p class="text-[11px] text-muted-foreground-2 mt-2 font-mono truncate">{rule.rule_id}</p>
                    <p class="text-[11px] text-muted-foreground-2 mt-1">{sourceLabel(rule)} · priority {rule.priority}</p>
                  </div>
                {/each}
              </div>
            </section>
            <section>
              <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2">Detection</h3>
              <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
                {#each detectionRules as rule (rule.rule_id)}
                  <div class="p-4">
                    <div class="flex items-start justify-between gap-x-3">
                      <div class="min-w-0">
                        <p class="text-sm font-medium text-foreground truncate">{rule.name}</p>
                        {#if rule.reason}
                          <p class="text-xs text-muted-foreground-1 mt-0.5 line-clamp-2">{rule.reason}</p>
                        {/if}
                      </div>
                      <span class="text-xs px-2 py-0.5 rounded-full bg-muted text-muted-foreground-1 shrink-0">{rule.detection_level ?? 'none'}</span>
                    </div>
                    <p class="text-[11px] text-muted-foreground-2 mt-2 font-mono truncate">{rule.rule_id}</p>
                    <p class="text-[11px] text-muted-foreground-2 mt-1">{sourceLabel(rule)} · priority {rule.priority}</p>
                  </div>
                {/each}
              </div>
            </section>
          </div>
        {:else if activeSection === 'plugins'}
          <PluginSection />
        {:else if activeSection === 'mcp'}
          <McpSection />
        {:else if activeSection === 'assets'}
          <h2 class="text-xl font-medium text-foreground mb-6">Assets</h2>
          <pre class="bg-card border border-card-line rounded-xl p-4 text-xs text-foreground overflow-auto">{JSON.stringify(assetsInfo, null, 2)}</pre>
        {/if}
      </div>
    {/if}
  </main>
</div>
