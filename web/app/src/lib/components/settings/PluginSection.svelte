<script lang="ts">
  import { onMount } from 'svelte';
  import CredentialInventory from './CredentialInventory.svelte';
  import ContentPluginControls from './ContentPluginControls.svelte';
  import { listPlugins, updatePlugin } from '../../api';
  import type {
    PluginDetectionLevel,
    PluginInfo,
    PluginListResponse,
    PluginMode,
    ContentFilterConfig,
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
      tone: 'text-destructive-text border-destructive/30 bg-destructive/10',
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

  function runtimeSummary(plugin: PluginInfo): string {
    const { runtime } = plugin;
    return `${runtime.event_count} events, ${runtime.detection_count} detections`;
  }

  function formatMicros(micros: number): string {
    if (micros < 1_000) return `${micros}us`;
    return `${(micros / 1_000).toFixed(1)}ms`;
  }

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

  async function setContent(plugin:PluginInfo,content:ContentFilterConfig) {
    saving={...saving,[plugin.id]:true};error=null;
    try {replacePlugin(await updatePlugin(plugin.id,{content}));}
    catch(err) {error=err instanceof Error?err.message:'Could not save inspection options';}
    finally {saving={...saving,[plugin.id]:false};}
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
  <div class="border border-destructive/40 rounded-lg p-4 text-sm text-destructive-text">
    {error}
  </div>
{:else if response}
  {#if error}
    <div class="border border-destructive/40 rounded-lg p-3 text-sm text-destructive-text mb-4">
      {error}
    </div>
  {/if}

  <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
    {#each response.plugins as plugin (plugin.id)}
      {#if plugin.id === 'credential_broker'}
        <div class="p-4"><CredentialInventory /></div>
      {:else}
      {@const modeMeta = pluginModeMeta(plugin.config.mode)}
      <div class="plugin-row p-4 {plugin.config.mode === 'disable' ? 'bg-muted/20 opacity-70' : ''}">
        <div class="plugin-grid">
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
            <p class="truncate">
              runs {plugin.runtime.execution_count} · applied {plugin.runtime.applied_count} · latency max {formatMicros(plugin.runtime.max_duration_us)}
            </p>
            {#if plugin.runtime.last_error}
              <p class="truncate text-destructive-text">{plugin.runtime.last_error}</p>
            {/if}
          </div>

          <label class="flex min-w-0 flex-col gap-1"><span class="text-xs text-muted-foreground-1">Mode</span><select
            class="py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary disabled:opacity-60"
            value={plugin.config.mode}
            disabled={saving[plugin.id]}
            aria-label="{plugin.id} mode"
            onchange={(e) => setMode(plugin, (e.target as HTMLSelectElement).value as PluginMode)}
          >
            {#if plugin.id==='utf8_guard' && plugin.config.mode==='rewrite'}<option value="rewrite" disabled>Rewrite (blocks invalid text)</option>{/if}
            {#each MODES.filter(mode=>plugin.id!=='utf8_guard' || mode.value!=='rewrite') as mode (mode.value)}
              <option value={mode.value}>{mode.label}</option>
            {/each}
          </select>
          </label>
          <label class="flex min-w-0 flex-col gap-1"><span class="text-xs text-muted-foreground-1">Detection level</span><select
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
          </label>
        </div>
        {#if ['pii_filter','secret_exfiltration','utf8_guard'].includes(plugin.id)}
          <ContentPluginControls pluginId={plugin.id} content={plugin.config.content??{direction:'both',email:true,phone:true,card:true,ssn:true}}
            disabled={saving[plugin.id]} onchange={content=>{void setContent(plugin,content);}}/>
        {/if}
      </div>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .plugin-row {container-type:inline-size;}
  .plugin-grid {display:grid;grid-template-columns:minmax(0,1fr);align-items:center;gap:.75rem;}
  @container (min-width:56rem) {
    .plugin-grid {grid-template-columns:minmax(0,1fr) minmax(10rem,14rem) 10rem 12rem;column-gap:1rem;}
  }
</style>
