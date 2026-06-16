<script lang="ts">
  import { getCredentialBrokerInfo, listPlugins, reloadCredentialBrokerStore, updatePlugin } from '../../api';
  import type {
    CredentialBrokerInfo,
    PluginDetectionLevel,
    PluginInfo,
    PluginListResponse,
    PluginMode,
  } from '../../api';
  import CheckCircle from 'phosphor-svelte/lib/CheckCircle';
  import HandPalm from 'phosphor-svelte/lib/HandPalm';
  import PencilSimple from 'phosphor-svelte/lib/PencilSimple';
  import Prohibit from 'phosphor-svelte/lib/Prohibit';
  import XCircle from 'phosphor-svelte/lib/XCircle';

  const MODES: { value: PluginMode; label: string }[] = [
    { value: 'allow', label: 'Allow' },
    { value: 'ask', label: 'Ask' },
    { value: 'block', label: 'Block' },
    { value: 'rewrite', label: 'Rewrite' },
    { value: 'disable', label: 'Disable' },
  ];

  const DETECTION_LEVELS: { value: PluginDetectionLevel; label: string }[] = [
    { value: 'informational', label: 'Informational' },
    { value: 'low', label: 'Low' },
    { value: 'medium', label: 'Medium' },
    { value: 'high', label: 'High' },
    { value: 'critical', label: 'Critical' },
  ];

  const MODE_META: Record<PluginMode, { label: string; icon: typeof CheckCircle; tone: string }> = {
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
    rewrite: {
      label: 'Rewrite',
      icon: PencilSimple,
      tone: 'text-primary border-primary/30 bg-primary/10',
    },
    disable: {
      label: 'Disabled',
      icon: XCircle,
      tone: 'text-muted-foreground-2 border-line-2 bg-muted/40',
    },
  };

  const STAGE_LABELS = {
    preprocess: 'Preprocess',
    postprocess: 'Postprocess',
    logging: 'Logging',
  };

  function pluginModeMeta(mode: PluginMode) {
    return MODE_META[mode];
  }

  let { profileId } = $props<{ profileId: string }>();

  function runtimeSummary(plugin: PluginInfo): string {
    const { runtime } = plugin;
    return `${runtime.event_count} events, ${runtime.detection_count} detections`;
  }

  let response = $state<PluginListResponse | null>(null);
  let credentialBrokerInfo = $state<CredentialBrokerInfo | null>(null);
  let loading = $state(true);
  let brokerLoading = $state(false);
  let saving = $state<Record<string, boolean>>({});
  let error = $state<string | null>(null);
  let brokerError = $state<string | null>(null);

  let loadedProfileId = $state<string | null>(null);

  $effect(() => {
    if (profileId && profileId !== loadedProfileId) {
      loadedProfileId = profileId;
      void load();
    }
  });

  async function load() {
    loading = true;
    error = null;
    brokerError = null;
    try {
      response = await listPlugins(profileId);
      const broker = response.plugins.find((plugin) => plugin.id === 'credential_broker');
      if (broker?.detail_routes.some((route) => route.kind === 'credential_broker')) {
        await loadCredentialBrokerInfo(response.scope.profile_id);
      }
    } catch (err) {
      error = String(err instanceof Error ? err.message : err);
    } finally {
      loading = false;
    }
  }

  async function loadCredentialBrokerInfo(activeProfileId = response?.scope.profile_id ?? profileId) {
    brokerLoading = true;
    brokerError = null;
    try {
      credentialBrokerInfo = await getCredentialBrokerInfo(activeProfileId);
    } catch (err) {
      credentialBrokerInfo = null;
      brokerError = String(err instanceof Error ? err.message : err);
    } finally {
      brokerLoading = false;
    }
  }

  async function retryCredentialBrokerStore(activeProfileId = response?.scope.profile_id ?? profileId) {
    brokerLoading = true;
    brokerError = null;
    try {
      credentialBrokerInfo = await reloadCredentialBrokerStore(activeProfileId);
    } catch (err) {
      brokerError = String(err instanceof Error ? err.message : err);
    } finally {
      brokerLoading = false;
    }
  }

  function replacePlugin(next: PluginInfo) {
    if (!response) return;
    response = {
      ...response,
      plugins: response.plugins.map((plugin) => plugin.id === next.id ? next : plugin),
    };
  }

  async function setMode(plugin: PluginInfo, mode: PluginMode) {
    saving = { ...saving, [plugin.id]: true };
    error = null;
    try {
      const activeProfileId = response?.scope.profile_id ?? profileId;
      replacePlugin(await updatePlugin(activeProfileId, plugin.id, { mode }));
      if (plugin.id === 'credential_broker') {
        await loadCredentialBrokerInfo(activeProfileId);
      }
    } catch (err) {
      error = String(err instanceof Error ? err.message : err);
    } finally {
      saving = { ...saving, [plugin.id]: false };
    }
  }

  async function setDetectionLevel(plugin: PluginInfo, detection_level: PluginDetectionLevel) {
    saving = { ...saving, [plugin.id]: true };
    error = null;
    try {
      replacePlugin(await updatePlugin(response?.scope.profile_id ?? profileId, plugin.id, { detection_level }));
    } catch (err) {
      error = String(err instanceof Error ? err.message : err);
    } finally {
      saving = { ...saving, [plugin.id]: false };
    }
  }
</script>

<h2 class="text-xl font-medium text-foreground mb-6">Plugins</h2>

{#if loading}
  <div class="flex items-center justify-center h-32">
    <div class="animate-spin size-6 border-2 border-primary border-t-transparent rounded-full"></div>
  </div>
{:else if error && !response}
  <div class="border border-destructive/40 rounded-lg p-4 text-sm text-destructive-foreground">
    {error}
  </div>
{:else if response}
  {#if error}
    <div class="border border-destructive/40 rounded-lg p-3 text-sm text-destructive-foreground mb-4">
      {error}
    </div>
  {/if}

  <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
    {#each response.plugins as plugin (plugin.id)}
      {@const modeMeta = pluginModeMeta(plugin.config.mode)}
      <div class="p-4 {plugin.config.mode === 'disable' ? 'bg-muted/20 opacity-70' : ''}">
        <div class="grid grid-cols-[minmax(0,1fr)_minmax(10rem,14rem)_10rem_12rem] items-center gap-x-4">
          <div class="min-w-0">
            <div class="flex items-center gap-x-2">
              <p class="text-sm font-medium text-foreground truncate">{plugin.name}</p>
              <span class={`inline-flex items-center gap-x-1 rounded-full border px-2 py-0.5 text-[11px] font-medium ${modeMeta.tone}`}>
                <modeMeta.icon size={12} weight="fill" />
                {modeMeta.label}
              </span>
              {#if plugin.overridden}
                <span class="text-[11px] uppercase tracking-wide text-primary">Overridden</span>
              {/if}
              {#if plugin.detail_routes.length > 0}
                <span class="text-[11px] uppercase tracking-wide text-muted-foreground-2">Details</span>
              {/if}
            </div>
            <p class="text-xs text-muted-foreground-1 mt-0.5 line-clamp-2">{plugin.description}</p>
            <p class="text-[11px] text-muted-foreground-2 mt-1">
              {STAGE_LABELS[plugin.stage]} · v{plugin.version}
            </p>
            {#if plugin.capabilities.event_families.length > 0}
              <div class="mt-2 flex flex-wrap gap-1">
                {#each plugin.capabilities.event_families as family (family)}
                  <span class="rounded-full border border-line-2 bg-muted/40 px-1.5 py-0.5 text-[10px] text-muted-foreground-1">
                    {family}
                  </span>
                {/each}
              </div>
            {/if}
          </div>

          <div class="min-w-0 text-xs text-muted-foreground-1">
            <p class="truncate">{runtimeSummary(plugin)}</p>
            <p class="truncate">blocks {plugin.runtime.block_count} · rewrites {plugin.runtime.rewrite_count}</p>
            {#if plugin.runtime.last_error}
              <p class="truncate text-destructive-foreground">{plugin.runtime.last_error}</p>
            {/if}
          </div>

          <select
            class="py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary disabled:opacity-60"
            value={plugin.config.mode}
            disabled={saving[plugin.id]}
            aria-label="{plugin.id} mode"
            onchange={(e) => setMode(plugin, (e.target as HTMLSelectElement).value as PluginMode)}
          >
            {#each MODES as mode (mode.value)}
              <option value={mode.value}>{mode.label}</option>
            {/each}
          </select>

          <select
            class="py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary disabled:opacity-60"
            value={plugin.config.detection_level}
            disabled={saving[plugin.id] || plugin.config.mode === 'disable'}
            aria-label="{plugin.id} detection level"
            onchange={(e) => setDetectionLevel(plugin, (e.target as HTMLSelectElement).value as PluginDetectionLevel)}
          >
            {#each DETECTION_LEVELS as level (level.value)}
              <option value={level.value}>{level.label}</option>
            {/each}
          </select>
        </div>

        {#if plugin.id === 'credential_broker' && plugin.detail_routes.some((route) => route.kind === 'credential_broker')}
          <div class="mt-4 border border-card-line rounded-lg bg-layer p-4">
            <div class="flex items-start justify-between gap-x-4">
              <div>
                <p class="text-sm font-medium text-foreground">Credential Broker</p>
                <p class="text-xs text-muted-foreground-1 mt-0.5">
                  {credentialBrokerInfo?.inventory.length ?? 0} credentials · profile {credentialBrokerInfo?.grants.profile_enabled ? 'enabled' : 'disabled'}
                </p>
              </div>
              <button
                type="button"
                class="py-1.5 px-3 text-xs font-medium rounded-md bg-muted text-foreground hover:bg-muted-hover disabled:opacity-60"
                disabled={brokerLoading}
                onclick={() => loadCredentialBrokerInfo(response?.scope.profile_id ?? profileId)}
              >
                Refresh
              </button>
              <button
                type="button"
                class="py-1.5 px-3 text-xs font-medium rounded-md bg-primary text-primary-foreground hover:bg-primary-hover disabled:opacity-60"
                disabled={brokerLoading}
                onclick={() => retryCredentialBrokerStore(response?.scope.profile_id ?? profileId)}
              >
                Retry store
              </button>
            </div>

            {#if brokerError}
              <p class="mt-3 text-xs text-destructive-foreground">{brokerError}</p>
            {:else if brokerLoading && !credentialBrokerInfo}
              <p class="mt-3 text-xs text-muted-foreground-1">Loading broker details...</p>
            {:else if credentialBrokerInfo}
              <div class="grid grid-cols-2 gap-3 mt-4">
                <div class="rounded-md border border-line-2 p-3">
                  <p class="text-[11px] uppercase tracking-wide text-muted-foreground-2">Store</p>
                  <p class="mt-2 text-xs text-foreground">
                    {credentialBrokerInfo.store.status} · {credentialBrokerInfo.store.backend} · {credentialBrokerInfo.store.cached_count} cached
                  </p>
                  {#if credentialBrokerInfo.store.last_error}
                    <p class="mt-1 text-xs text-destructive-foreground">{credentialBrokerInfo.store.last_error}</p>
                  {/if}
                </div>
                <div class="rounded-md border border-line-2 p-3">
                  <p class="text-[11px] uppercase tracking-wide text-muted-foreground-2">Supported providers</p>
                  <p class="mt-2 text-xs text-foreground">
                    {plugin.capabilities.credential_providers.join(', ') || 'none'}
                  </p>
                </div>
              </div>

              <div class="grid grid-cols-2 gap-3 mt-4">
                <div class="rounded-md border border-line-2 p-3">
                  <p class="text-[11px] uppercase tracking-wide text-muted-foreground-2">Credential sources</p>
                  <p class="mt-2 text-xs text-foreground">
                    {plugin.capabilities.credential_sources.join(', ') || 'none'}
                  </p>
                </div>
              </div>

              <div class="grid grid-cols-3 gap-3 mt-4">
                <div class="rounded-md border border-line-2 p-3">
                  <p class="text-[11px] uppercase tracking-wide text-muted-foreground-2">Inventory</p>
                  <p class="text-lg font-semibold text-foreground">{credentialBrokerInfo.inventory.length}</p>
                </div>
                <div class="rounded-md border border-line-2 p-3">
                  <p class="text-[11px] uppercase tracking-wide text-muted-foreground-2">VM grants</p>
                  <p class="text-lg font-semibold text-foreground">{credentialBrokerInfo.grants.vm_grants.length}</p>
                </div>
                <div class="rounded-md border border-line-2 p-3">
                  <p class="text-[11px] uppercase tracking-wide text-muted-foreground-2">Corp constraints</p>
                  <p class="text-lg font-semibold text-foreground">{credentialBrokerInfo.corp_constraints.length}</p>
                </div>
              </div>

              {#if credentialBrokerInfo.inventory.length > 0}
                <ul class="mt-4 divide-y divide-card-divider border border-line-2 rounded-md">
                  {#each credentialBrokerInfo.inventory as credential (credential.credential_ref)}
                    <li class="grid grid-cols-[minmax(0,1fr)_6rem_6rem] gap-x-3 p-3 text-xs">
                      <div class="min-w-0">
                        <p class="font-mono text-foreground truncate">{credential.credential_ref}</p>
                        <p class="text-muted-foreground-2 truncate">{credential.provider ?? 'unknown'} · {credential.last_seen ?? 'never'}</p>
                      </div>
                      <p class="text-muted-foreground-1">{credential.observed_count} seen</p>
                      <p class="text-muted-foreground-1">{credential.injected_count} used</p>
                    </li>
                  {/each}
                </ul>
              {:else}
                <p class="mt-4 text-xs text-muted-foreground-1">No brokered credentials recorded for this profile.</p>
              {/if}

              {#if credentialBrokerInfo.corp_constraints.length > 0}
                <ul class="mt-4 space-y-2">
                  {#each credentialBrokerInfo.corp_constraints as constraint (constraint.id)}
                    <li class="text-xs text-muted-foreground-1">
                      <span class="font-medium text-foreground">{constraint.id}</span>
                      {constraint.description}
                    </li>
                  {/each}
                </ul>
              {/if}
            {/if}
          </div>
        {/if}
      </div>
    {/each}
  </div>
{/if}
