<script lang="ts">
  import { onMount } from 'svelte';
  import { getDebugReport } from '../../api';
  import type {
    RuntimeSecurityEngineReport,
    RuntimeSecurityRegistryReport,
  } from '../../types/gateway';
  import ArrowClockwise from 'phosphor-svelte/lib/ArrowClockwise';
  import ShieldCheck from 'phosphor-svelte/lib/ShieldCheck';
  import WarningCircle from 'phosphor-svelte/lib/WarningCircle';

  let loading = $state(false);
  let error = $state<string | null>(null);
  let engine = $state<RuntimeSecurityEngineReport | null>(null);

  async function refreshHealth() {
    loading = true;
    error = null;
    try {
      const report = await getDebugReport();
      if (!report.json?.security_engine) {
        throw new Error('Security engine health is unavailable in the debug report.');
      }
      engine = report.json.security_engine;
    } catch (err) {
      error = String(err instanceof Error ? err.message : err);
      engine = null;
    } finally {
      loading = false;
    }
  }

  function registryHealthLabel(registry: RuntimeSecurityRegistryReport): string {
    if (registry.error_count > 0) return `${registry.error_count} compile error${registry.error_count === 1 ? '' : 's'}`;
    return `${registry.compiled_count}/${registry.rule_count} compiled`;
  }

  onMount(() => {
    refreshHealth();
  });
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between gap-x-4">
    <div>
      <h2 class="text-xl font-medium text-foreground">Security Engine Health</h2>
      <p class="text-sm text-muted-foreground-1 mt-0.5">Authoritative runtime rule, match, and confirm state.</p>
    </div>
    <button
      type="button"
      class="p-2 rounded-lg border border-line-2 bg-layer text-foreground hover:bg-layer-hover transition-colors disabled:opacity-60"
      title="Refresh security health"
      aria-label="Refresh security health"
      disabled={loading}
      onclick={refreshHealth}
    >
      <ArrowClockwise size={16} />
    </button>
  </div>

  {#if loading && !engine}
    <div class="bg-card border border-card-line rounded-xl p-6 text-center">
      <p class="text-sm text-muted-foreground-1">Loading security health...</p>
    </div>
  {:else if error}
    <div class="bg-card border border-card-line rounded-xl p-4 flex items-start gap-x-3">
      <WarningCircle class="shrink-0 text-destructive" size={18} />
      <p class="text-sm text-destructive">{error}</p>
    </div>
  {:else if engine}
    <div class="grid grid-cols-1 lg:grid-cols-3 gap-3">
      <article class="bg-card border border-card-line rounded-xl p-4">
        <div class="flex items-start justify-between gap-x-3">
          <div>
            <p class="text-xs font-semibold text-foreground uppercase tracking-wider">Enforcement</p>
            <p class="text-2xl font-semibold text-foreground mt-2">{engine.enforcement.rule_count}</p>
          </div>
          <ShieldCheck class="text-primary" size={20} />
        </div>
        <dl class="grid grid-cols-2 gap-x-4 gap-y-2 mt-4 text-xs">
          <dt class="text-muted-foreground-1">Enabled</dt>
          <dd class="text-right text-foreground">{engine.enforcement.enabled_count}</dd>
          <dt class="text-muted-foreground-1">Compiled</dt>
          <dd class="text-right text-foreground">{registryHealthLabel(engine.enforcement)}</dd>
          <dt class="text-muted-foreground-1">Matches</dt>
          <dd class="text-right text-foreground">{engine.enforcement.match_count_total}</dd>
          <dt class="text-muted-foreground-1">Profile rules</dt>
          <dd class="text-right text-foreground">{engine.enforcement.profile_scope_count}</dd>
        </dl>
      </article>

      <article class="bg-card border border-card-line rounded-xl p-4">
        <div class="flex items-start justify-between gap-x-3">
          <div>
            <p class="text-xs font-semibold text-foreground uppercase tracking-wider">Detection</p>
            <p class="text-2xl font-semibold text-foreground mt-2">{engine.detection.rule_count}</p>
          </div>
          <ShieldCheck class="text-primary" size={20} />
        </div>
        <dl class="grid grid-cols-2 gap-x-4 gap-y-2 mt-4 text-xs">
          <dt class="text-muted-foreground-1">Enabled</dt>
          <dd class="text-right text-foreground">{engine.detection.enabled_count}</dd>
          <dt class="text-muted-foreground-1">Compiled</dt>
          <dd class="text-right text-foreground">{registryHealthLabel(engine.detection)}</dd>
          <dt class="text-muted-foreground-1">Findings</dt>
          <dd class="text-right text-foreground">{engine.detection.match_count_total}</dd>
          <dt class="text-muted-foreground-1">Runtime rules</dt>
          <dd class="text-right text-foreground">{engine.detection.runtime_scope_count}</dd>
        </dl>
      </article>

      <article class="bg-card border border-card-line rounded-xl p-4">
        <p class="text-xs font-semibold text-foreground uppercase tracking-wider">Runtime Contract</p>
        <dl class="grid grid-cols-2 gap-x-4 gap-y-2 mt-4 text-xs">
          <dt class="text-muted-foreground-1">Rule store</dt>
          <dd class="text-right text-foreground">{engine.runtime_rules_store_enabled ? 'enabled' : 'disabled'}</dd>
          <dt class="text-muted-foreground-1">Confirm resolver</dt>
          <dd class="text-right text-foreground">{engine.confirm.resolver_available ? 'available' : 'unavailable'}</dd>
          <dt class="text-muted-foreground-1">Owner</dt>
          <dd class="text-right text-foreground">{engine.confirm.owner ?? 'none'}</dd>
        </dl>
        {#if engine.runtime_rules_store_path}
          <p class="text-xs font-mono text-muted-foreground-1 mt-4 break-all">{engine.runtime_rules_store_path}</p>
        {/if}
      </article>
    </div>
  {/if}
</div>
