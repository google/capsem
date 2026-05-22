<script lang="ts">
  import { onMount } from 'svelte';
  import {
    deleteRuntimeDetectionRule,
    deleteRuntimeEnforcementRule,
    getRuntimeDetectionRules,
    getRuntimeEnforcementRules,
    installRuntimeDetectionRule,
    installRuntimeEnforcementRule,
    validateRuntimeDetectionRule,
    validateRuntimeEnforcementRule,
  } from '../../api';
  import type {
    RuntimeConfidence,
    RuntimeDetectionRuleRequest,
    RuntimeEnforcementRuleRequest,
    RuntimeRuleEntry,
    RuntimeRuleKind,
    RuntimeSecurityDecision,
    RuntimeSeverity,
  } from '../../types/gateway';
  import ArrowClockwise from 'phosphor-svelte/lib/ArrowClockwise';
  import CheckCircle from 'phosphor-svelte/lib/CheckCircle';
  import Plus from 'phosphor-svelte/lib/Plus';
  import Trash from 'phosphor-svelte/lib/Trash';

  const DECISIONS: RuntimeSecurityDecision[] = ['allow', 'ask', 'block', 'rewrite', 'throttle'];
  const SEVERITIES: RuntimeSeverity[] = ['info', 'low', 'medium', 'high', 'critical'];
  const CONFIDENCES: RuntimeConfidence[] = ['low', 'medium', 'high'];

  type Draft = {
    id: string;
    packId: string;
    condition: string;
    priority: number;
    enabled: boolean;
    decision: RuntimeSecurityDecision;
    reason: string;
    sigmaId: string;
    title: string;
    severity: RuntimeSeverity;
    confidence: RuntimeConfidence;
    tags: string;
  };

  let activeKind = $state<RuntimeRuleKind>('enforcement');
  let enforcementRules = $state<RuntimeRuleEntry[]>([]);
  let detectionRules = $state<RuntimeRuleEntry[]>([]);
  let loading = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let statusMessage = $state<string | null>(null);
  let draft = $state<Draft>(emptyDraft());

  let activeRules = $derived(activeKind === 'enforcement' ? enforcementRules : detectionRules);
  let draftValid = $derived(draft.id.trim().length > 0 && draft.condition.trim().length > 0);

  function emptyDraft(): Draft {
    return {
      id: '',
      packId: 'runtime',
      condition: "http.request.host.contains('google')",
      priority: 100,
      enabled: true,
      decision: 'block',
      reason: '',
      sigmaId: '',
      title: '',
      severity: 'medium',
      confidence: 'high',
      tags: '',
    };
  }

  function tagsFromDraft(): string[] {
    return draft.tags
      .split(',')
      .map((tag) => tag.trim())
      .filter(Boolean);
  }

  function enforcementRequest(): RuntimeEnforcementRuleRequest {
    return {
      id: draft.id.trim(),
      pack_id: draft.packId.trim() || null,
      priority: Number(draft.priority),
      condition: draft.condition.trim(),
      decision: draft.decision,
      reason: draft.reason.trim() || null,
      enabled: draft.enabled,
    };
  }

  function detectionRequest(): RuntimeDetectionRuleRequest {
    return {
      id: draft.id.trim(),
      pack_id: draft.packId.trim() || 'runtime-detection',
      sigma_id: draft.sigmaId.trim() || null,
      title: draft.title.trim() || draft.id.trim(),
      priority: Number(draft.priority),
      condition: draft.condition.trim(),
      severity: draft.severity,
      confidence: draft.confidence,
      tags: tagsFromDraft(),
      enabled: draft.enabled,
    };
  }

  async function refreshRules() {
    loading = true;
    error = null;
    try {
      const [enforcement, detection] = await Promise.all([
        getRuntimeEnforcementRules(),
        getRuntimeDetectionRules(),
      ]);
      enforcementRules = enforcement.rules;
      detectionRules = detection.rules;
    } catch (err) {
      error = String(err instanceof Error ? err.message : err);
    } finally {
      loading = false;
    }
  }

  async function validateDraft() {
    if (!draftValid) return;
    busy = true;
    error = null;
    statusMessage = null;
    try {
      const result = activeKind === 'enforcement'
        ? await validateRuntimeEnforcementRule(enforcementRequest())
        : await validateRuntimeDetectionRule(detectionRequest());
      statusMessage = `${result.id} ${result.compiled ? 'compiled' : 'did not compile'}.`;
    } catch (err) {
      error = String(err instanceof Error ? err.message : err);
    } finally {
      busy = false;
    }
  }

  async function installDraft() {
    if (!draftValid) return;
    busy = true;
    error = null;
    statusMessage = null;
    try {
      const result = activeKind === 'enforcement'
        ? await installRuntimeEnforcementRule(enforcementRequest())
        : await installRuntimeDetectionRule(detectionRequest());
      statusMessage = `${result.rule.id} installed.`;
      draft = emptyDraft();
      await refreshRules();
    } catch (err) {
      error = String(err instanceof Error ? err.message : err);
    } finally {
      busy = false;
    }
  }

  async function deleteRule(rule: RuntimeRuleEntry) {
    if (rule.scope !== 'runtime') return;
    busy = true;
    error = null;
    statusMessage = null;
    try {
      if (activeKind === 'enforcement') {
        await deleteRuntimeEnforcementRule(rule.id);
      } else {
        await deleteRuntimeDetectionRule(rule.id);
      }
      statusMessage = `${rule.id} deleted.`;
      await refreshRules();
    } catch (err) {
      error = String(err instanceof Error ? err.message : err);
    } finally {
      busy = false;
    }
  }

  function decisionLabel(rule: RuntimeRuleEntry): string {
    return rule.definition.kind === 'enforcement' ? rule.definition.decision : rule.definition.severity;
  }

  function ruleTitle(rule: RuntimeRuleEntry): string | null {
    if (rule.definition.kind === 'detection') return rule.definition.title;
    return rule.definition.reason ?? null;
  }

  onMount(() => {
    refreshRules();
  });
</script>

<div class="space-y-4">
  <div class="flex items-center justify-between gap-x-4">
    <div>
      <h2 class="text-xl font-medium text-foreground">Live Rules</h2>
      <p class="text-sm text-muted-foreground-1 mt-0.5">Runtime enforcement and detection overlays.</p>
    </div>
    <button
      type="button"
      class="p-2 rounded-lg border border-line-2 bg-layer text-foreground hover:bg-layer-hover transition-colors disabled:opacity-60"
      title="Refresh rules"
      aria-label="Refresh rules"
      disabled={loading}
      onclick={refreshRules}
    >
      <ArrowClockwise size={16} />
    </button>
  </div>

  <div class="flex items-center gap-x-1">
    <button
      type="button"
      class="py-2 px-3 text-sm font-medium rounded-lg border
        {activeKind === 'enforcement'
          ? 'bg-primary border-primary-line text-primary-foreground'
          : 'bg-layer border-layer-line text-layer-foreground hover:bg-layer-hover'}"
      onclick={() => activeKind = 'enforcement'}
    >
      Enforcement
    </button>
    <button
      type="button"
      class="py-2 px-3 text-sm font-medium rounded-lg border
        {activeKind === 'detection'
          ? 'bg-primary border-primary-line text-primary-foreground'
          : 'bg-layer border-layer-line text-layer-foreground hover:bg-layer-hover'}"
      onclick={() => activeKind = 'detection'}
    >
      Detection
    </button>
  </div>

  <div>
    <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2">Add {activeKind} rule</h3>
    <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
      <div class="grid grid-cols-1 lg:grid-cols-[1fr_1fr_8rem] gap-3 p-4">
        <label class="block">
          <span class="text-xs font-medium text-foreground">Rule id</span>
          <input
            class="mt-1 w-full py-2 px-3 text-sm font-mono rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
            value={draft.id}
            oninput={(e) => draft = { ...draft, id: (e.target as HTMLInputElement).value }}
            placeholder={activeKind === 'enforcement' ? 'runtime-block-google' : 'runtime-detect-google'}
          />
        </label>
        <label class="block">
          <span class="text-xs font-medium text-foreground">Pack id</span>
          <input
            class="mt-1 w-full py-2 px-3 text-sm font-mono rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
            value={draft.packId}
            oninput={(e) => draft = { ...draft, packId: (e.target as HTMLInputElement).value }}
            placeholder="runtime"
          />
        </label>
        <label class="block">
          <span class="text-xs font-medium text-foreground">Priority</span>
          <input
            type="number"
            class="mt-1 w-full py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
            value={draft.priority}
            oninput={(e) => draft = { ...draft, priority: Number((e.target as HTMLInputElement).value) }}
          />
        </label>
      </div>

      <div class="p-4">
        <label class="block">
          <span class="text-xs font-medium text-foreground">Condition</span>
          <input
            class="mt-1 w-full py-2 px-3 text-sm font-mono rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
            value={draft.condition}
            oninput={(e) => draft = { ...draft, condition: (e.target as HTMLInputElement).value }}
            placeholder="http.request.host.contains('google')"
          />
        </label>
      </div>

      {#if activeKind === 'enforcement'}
        <div class="grid grid-cols-1 lg:grid-cols-[12rem_1fr] gap-3 p-4">
          <label class="block">
            <span class="text-xs font-medium text-foreground">Decision</span>
            <select
              class="mt-1 w-full py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
              value={draft.decision}
              onchange={(e) => draft = { ...draft, decision: (e.target as HTMLSelectElement).value as RuntimeSecurityDecision }}
            >
              {#each DECISIONS as decision (decision)}
                <option value={decision}>{decision}</option>
              {/each}
            </select>
          </label>
          <label class="block">
            <span class="text-xs font-medium text-foreground">Reason</span>
            <input
              class="mt-1 w-full py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
              value={draft.reason}
              oninput={(e) => draft = { ...draft, reason: (e.target as HTMLInputElement).value }}
              placeholder="Short audit reason"
            />
          </label>
        </div>
      {:else}
        <div class="grid grid-cols-1 lg:grid-cols-4 gap-3 p-4">
          <label class="block">
            <span class="text-xs font-medium text-foreground">Title</span>
            <input
              class="mt-1 w-full py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
              value={draft.title}
              oninput={(e) => draft = { ...draft, title: (e.target as HTMLInputElement).value }}
              placeholder="Secret egress"
            />
          </label>
          <label class="block">
            <span class="text-xs font-medium text-foreground">Sigma id</span>
            <input
              class="mt-1 w-full py-2 px-3 text-sm font-mono rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
              value={draft.sigmaId}
              oninput={(e) => draft = { ...draft, sigmaId: (e.target as HTMLInputElement).value }}
              placeholder="capsem-secret-egress"
            />
          </label>
          <label class="block">
            <span class="text-xs font-medium text-foreground">Severity</span>
            <select
              class="mt-1 w-full py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
              value={draft.severity}
              onchange={(e) => draft = { ...draft, severity: (e.target as HTMLSelectElement).value as RuntimeSeverity }}
            >
              {#each SEVERITIES as severity (severity)}
                <option value={severity}>{severity}</option>
              {/each}
            </select>
          </label>
          <label class="block">
            <span class="text-xs font-medium text-foreground">Confidence</span>
            <select
              class="mt-1 w-full py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
              value={draft.confidence}
              onchange={(e) => draft = { ...draft, confidence: (e.target as HTMLSelectElement).value as RuntimeConfidence }}
            >
              {#each CONFIDENCES as confidence (confidence)}
                <option value={confidence}>{confidence}</option>
              {/each}
            </select>
          </label>
          <label class="block lg:col-span-4">
            <span class="text-xs font-medium text-foreground">Tags</span>
            <input
              class="mt-1 w-full py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
              value={draft.tags}
              oninput={(e) => draft = { ...draft, tags: (e.target as HTMLInputElement).value }}
              placeholder="http, egress"
            />
          </label>
        </div>
      {/if}

      <div class="p-4 flex items-center justify-between gap-x-4">
        <label class="inline-flex items-center gap-x-2">
          <input
            type="checkbox"
            class="rounded border-line-2 text-primary focus:ring-primary"
            checked={draft.enabled}
            onchange={(e) => draft = { ...draft, enabled: (e.target as HTMLInputElement).checked }}
          />
          <span class="text-sm text-foreground">Enabled</span>
        </label>
        <div class="flex items-center gap-x-2">
          <button
            type="button"
            class="py-2 px-4 inline-flex items-center gap-x-1.5 text-sm font-medium rounded-lg border border-line-2 bg-layer text-foreground hover:bg-layer-hover transition-colors disabled:opacity-50"
            disabled={!draftValid || busy}
            onclick={validateDraft}
          >
            <CheckCircle size={16} />
            Validate
          </button>
          <button
            type="button"
            class="py-2 px-4 inline-flex items-center gap-x-1.5 text-sm font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover transition-colors disabled:opacity-50"
            disabled={!draftValid || busy}
            onclick={installDraft}
          >
            <Plus size={16} />
            Install
          </button>
        </div>
      </div>
    </div>
    {#if error}
      <p class="text-xs text-destructive mt-2">{error}</p>
    {:else if statusMessage}
      <p class="text-xs text-primary mt-2">{statusMessage}</p>
    {/if}
  </div>

  <div>
    <div class="flex items-center justify-between mb-2">
      <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider">Active {activeKind} rules</h3>
      <span class="text-xs text-muted-foreground-1">{activeRules.length} rule{activeRules.length === 1 ? '' : 's'}</span>
    </div>

    {#if loading}
      <div class="bg-card border border-card-line rounded-xl p-6 text-center">
        <p class="text-sm text-muted-foreground-1">Loading rules...</p>
      </div>
    {:else if activeRules.length === 0}
      <div class="bg-card border border-card-line rounded-xl p-6 text-center">
        <p class="text-sm text-muted-foreground-1">No active {activeKind} rules.</p>
      </div>
    {:else}
      <div class="space-y-2">
        {#each activeRules as rule (rule.id)}
          <article class="bg-card border border-card-line rounded-xl p-4">
            <div class="flex items-start justify-between gap-x-4">
              <div class="min-w-0">
                <div class="flex items-center gap-x-2 flex-wrap">
                  <span class="text-sm font-mono text-foreground">{rule.id}</span>
                  <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground-1">{rule.scope}</span>
                  <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground-1">{rule.origin}</span>
                  <span class="text-[10px] px-1.5 py-0.5 rounded-full {decisionLabel(rule) === 'block' || decisionLabel(rule) === 'critical' || decisionLabel(rule) === 'high' ? 'bg-destructive/10 text-destructive' : 'bg-primary/10 text-primary'}">{decisionLabel(rule)}</span>
                  <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground-1">priority {rule.priority}</span>
                  <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground-1">{rule.match_count} match{rule.match_count === 1 ? '' : 'es'}</span>
                  {#if !rule.enabled}
                    <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-warning/10 text-warning">disabled</span>
                  {/if}
                  {#if !rule.compiled}
                    <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-destructive/10 text-destructive">uncompiled</span>
                  {/if}
                </div>
                <p class="text-xs font-mono text-muted-foreground-1 mt-1 break-all">{rule.condition}</p>
                {#if ruleTitle(rule)}
                  <p class="text-xs text-muted-foreground-1 mt-1">{ruleTitle(rule)}</p>
                {/if}
                {#if rule.pack_id}
                  <p class="text-xs text-muted-foreground-1 mt-1">{rule.pack_id}</p>
                {/if}
              </div>
              <button
                type="button"
                class="p-1.5 rounded-md text-muted-foreground-1 hover:text-destructive hover:bg-muted-hover transition-colors disabled:opacity-40 disabled:hover:text-muted-foreground-1 disabled:hover:bg-transparent"
                title={rule.scope === 'runtime' ? 'Delete runtime rule' : 'Profile-owned rule'}
                aria-label={rule.scope === 'runtime' ? `Delete ${rule.id}` : `Delete ${rule.id} disabled`}
                disabled={rule.scope !== 'runtime' || busy}
                onclick={() => deleteRule(rule)}
              >
                <Trash size={16} />
              </button>
            </div>
          </article>
        {/each}
      </div>
    {/if}
  </div>
</div>
