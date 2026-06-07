<script lang="ts">
  import { onMount } from 'svelte';
  import { listPlugins, updatePlugin } from '../../api';
  import type {
    PluginDetectionLevel,
    PluginInfo,
    PluginListResponse,
    PluginMode,
  } from '../../api';

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

  let response = $state<PluginListResponse | null>(null);
  let loading = $state(true);
  let saving = $state<Record<string, boolean>>({});
  let error = $state<string | null>(null);

  onMount(() => {
    void load();
  });

  async function load() {
    loading = true;
    error = null;
    try {
      response = await listPlugins();
    } catch (err) {
      error = String(err instanceof Error ? err.message : err);
    } finally {
      loading = false;
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
      replacePlugin(await updatePlugin(plugin.id, { mode }));
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
      replacePlugin(await updatePlugin(plugin.id, { detection_level }));
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
      <div class="grid grid-cols-[minmax(0,1fr)_10rem_12rem] items-center gap-x-4 p-4">
        <div class="min-w-0">
          <div class="flex items-center gap-x-2">
            <p class="text-sm font-medium text-foreground truncate">{plugin.id}</p>
            {#if plugin.overridden}
              <span class="text-[11px] uppercase tracking-wide text-primary">Overridden</span>
            {/if}
          </div>
          <p class="text-xs text-muted-foreground-1 mt-0.5 line-clamp-2">{plugin.description}</p>
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
    {/each}
  </div>
{/if}
