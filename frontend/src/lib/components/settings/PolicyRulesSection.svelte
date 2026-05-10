<script lang="ts">
  import { settingsStore } from '../../stores/settings.svelte.ts';
  import {
    POLICY_RULE_TYPES,
    policyRuleNameFromParts,
    type PolicyRuleEntry,
    type PolicyRuleType,
  } from '../../models/settings-model';
  import type { PolicyCallback, PolicyDecisionKind, PolicyRuleConfig } from '../../types/settings';
  import Plus from 'phosphor-svelte/lib/Plus';
  import Trash from 'phosphor-svelte/lib/Trash';
  import X from 'phosphor-svelte/lib/X';

  const DECISIONS: PolicyDecisionKind[] = ['allow', 'ask', 'block', 'rewrite'];

  type RuleDraft = {
    type: PolicyRuleType;
    name: string;
    on: PolicyCallback;
    condition: string;
    decision: PolicyDecisionKind;
    priority: number;
    reason: string;
    rewriteTarget: string;
    rewriteValue: string;
    stripRequestHeaders: string;
    stripResponseHeaders: string;
  };

  let activeType = $state<PolicyRuleType>('http');
  let editingKey = $state<string | null>(null);
  let stagedMessage = $state<string | null>(null);
  let draft = $state<RuleDraft>(emptyDraft('http'));

  let entries = $derived(settingsStore.model?.policyRuleEntries ?? []);
  let generatedEntries = $derived(settingsStore.model?.generatedPolicyRuleEntries ?? []);

  let visibleEntries = $derived.by(() => entries.filter((entry) => entry.type === activeType));

  function callbacksFor(type: PolicyRuleType): PolicyCallback[] {
    return settingsStore.model?.callbacksForPolicyType(type) ?? ['http.request'];
  }

  function emptyDraft(type: PolicyRuleType): RuleDraft {
    return {
      type,
      name: '',
      on: callbacksFor(type)[0],
      condition: type === 'http' ? 'request.host == "example.com"' : '',
      decision: type === 'http' ? 'block' : 'ask',
      priority: 100,
      reason: '',
      rewriteTarget: '',
      rewriteValue: '',
      stripRequestHeaders: '',
      stripResponseHeaders: '',
    };
  }

  function onTypeChange(type: PolicyRuleType) {
    activeType = type;
    editingKey = null;
    draft = emptyDraft(type);
  }

  function csvToList(value: string): string[] {
    return value
      .split(',')
      .map((part) => part.trim())
      .filter(Boolean);
  }

  function listToCsv(value: string[] | undefined): string {
    return (value ?? []).join(', ');
  }

  function normalizeRuleName(name: string): string {
    return policyRuleNameFromParts([name]);
  }

  function ruleFromDraft(): PolicyRuleConfig {
    const rule: PolicyRuleConfig = {
      on: draft.on,
      if: draft.condition.trim(),
      decision: draft.decision,
      priority: Number(draft.priority),
    };
    if (draft.reason.trim()) {
      rule.reason = draft.reason.trim();
    }
    if (draft.decision === 'rewrite') {
      if (draft.rewriteTarget.trim()) {
        rule.rewrite_target = draft.rewriteTarget.trim();
      }
      if (draft.rewriteValue.trim()) {
        rule.rewrite_value = draft.rewriteValue.trim();
      }
      const stripRequest = csvToList(draft.stripRequestHeaders);
      const stripResponse = csvToList(draft.stripResponseHeaders);
      if (stripRequest.length > 0) rule.strip_request_headers = stripRequest;
      if (stripResponse.length > 0) rule.strip_response_headers = stripResponse;
    }
    return rule;
  }

  function editRule(entry: PolicyRuleEntry) {
    activeType = entry.type;
    editingKey = entry.key;
    draft = {
      type: entry.type,
      name: entry.name,
      on: entry.rule.on,
      condition: entry.rule.if,
      decision: entry.rule.decision,
      priority: entry.rule.priority,
      reason: entry.rule.reason ?? '',
      rewriteTarget: entry.rule.rewrite_target ?? '',
      rewriteValue: entry.rule.rewrite_value ?? '',
      stripRequestHeaders: listToCsv(entry.rule.strip_request_headers),
      stripResponseHeaders: listToCsv(entry.rule.strip_response_headers),
    };
    stagedMessage = null;
  }

  function cancelEdit() {
    editingKey = null;
    draft = emptyDraft(activeType);
    stagedMessage = null;
  }

  function stageDraft() {
    const name = normalizeRuleName(draft.name);
    if (!name || !draft.condition.trim()) return;
    settingsStore.stagePolicyRule(draft.type, name, ruleFromDraft());
    stagedMessage = `${editingKey ? 'Updated' : 'Added'} ${draft.type}.${name}.`;
    editingKey = null;
    draft = emptyDraft(activeType);
  }

  function deleteRule(entry: PolicyRuleEntry) {
    settingsStore.deletePolicyRule(entry.type, entry.name);
    stagedMessage = `Deleted ${entry.type}.${entry.name}.`;
    if (editingKey === entry.key) cancelEdit();
  }

  function stageGenerated(entry: PolicyRuleEntry) {
    settingsStore.stagePolicyRule(entry.type, entry.name, entry.rule);
    stagedMessage = `Generated ${entry.type}.${entry.name}.`;
  }

  function stageAllGenerated() {
    const count = settingsStore.stageGeneratedPolicyRules();
    stagedMessage = `${count} generated rule${count === 1 ? '' : 's'} staged.`;
  }
</script>

<div class="space-y-6">
  <div>
    <h2 class="text-xl font-medium text-foreground">Policy Rules</h2>
    <p class="text-sm text-muted-foreground-1 mt-0.5">Named rules saved as policy.&lt;type&gt;.&lt;rule_name&gt;.</p>
  </div>

  <div class="flex items-center gap-x-1">
    {#each POLICY_RULE_TYPES as type (type)}
      <button
        type="button"
        class="py-2 px-3 text-sm font-medium rounded-lg border capitalize
          {activeType === type
            ? 'bg-primary border-primary-line text-primary-foreground'
            : 'bg-layer border-layer-line text-layer-foreground hover:bg-layer-hover'}"
        onclick={() => onTypeChange(type)}
      >
        {type}
      </button>
    {/each}
  </div>

  <div>
    <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2">{editingKey ? 'Edit Rule' : 'Add Rule'}</h3>
    <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
      <div class="grid grid-cols-1 lg:grid-cols-4 gap-3 p-4">
        <label class="block">
          <span class="text-xs font-medium text-foreground">Type</span>
          <select
            class="mt-1 w-full py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
            value={draft.type}
            onchange={(e) => {
              const type = (e.target as HTMLSelectElement).value as PolicyRuleType;
              activeType = type;
              draft = { ...draft, type, on: callbacksFor(type)[0] };
            }}
          >
            {#each POLICY_RULE_TYPES as type (type)}
              <option value={type}>{type}</option>
            {/each}
          </select>
        </label>
        <label class="block">
          <span class="text-xs font-medium text-foreground">Callback</span>
          <select
            class="mt-1 w-full py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
            value={draft.on}
            onchange={(e) => draft = { ...draft, on: (e.target as HTMLSelectElement).value as PolicyCallback }}
          >
            {#each callbacksFor(draft.type) as callback (callback)}
              <option value={callback}>{callback}</option>
            {/each}
          </select>
        </label>
        <label class="block">
          <span class="text-xs font-medium text-foreground">Name</span>
          <input
            class="mt-1 w-full py-2 px-3 text-sm font-mono rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
            value={draft.name}
            oninput={(e) => draft = { ...draft, name: (e.target as HTMLInputElement).value }}
            placeholder="block_prod_token"
          />
        </label>
        <label class="block">
          <span class="text-xs font-medium text-foreground">Decision</span>
          <select
            class="mt-1 w-full py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
            value={draft.decision}
            onchange={(e) => draft = { ...draft, decision: (e.target as HTMLSelectElement).value as PolicyDecisionKind }}
          >
            {#each DECISIONS as decision (decision)}
              <option value={decision}>{decision}</option>
            {/each}
          </select>
        </label>
      </div>

      <div class="p-4 grid grid-cols-1 lg:grid-cols-[1fr_8rem] gap-3">
        <label class="block">
          <span class="text-xs font-medium text-foreground">Condition</span>
          <input
            class="mt-1 w-full py-2 px-3 text-sm font-mono rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
            value={draft.condition}
            oninput={(e) => draft = { ...draft, condition: (e.target as HTMLInputElement).value }}
            placeholder='request.host == "github.com"'
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

      {#if draft.decision === 'rewrite'}
        <div class="p-4 grid grid-cols-1 lg:grid-cols-2 gap-3">
          <label class="block">
            <span class="text-xs font-medium text-foreground">Rewrite target</span>
            <input
              class="mt-1 w-full py-2 px-3 text-sm font-mono rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
              value={draft.rewriteTarget}
              oninput={(e) => draft = { ...draft, rewriteTarget: (e.target as HTMLInputElement).value }}
              placeholder='response.text =~ "(?P&lt;secret&gt;sk-[A-Za-z0-9]+)"'
            />
          </label>
          <label class="block">
            <span class="text-xs font-medium text-foreground">Rewrite value</span>
            <input
              class="mt-1 w-full py-2 px-3 text-sm font-mono rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
              value={draft.rewriteValue}
              oninput={(e) => draft = { ...draft, rewriteValue: (e.target as HTMLInputElement).value }}
              placeholder="[redacted by capsem policy]"
            />
          </label>
          <label class="block">
            <span class="text-xs font-medium text-foreground">Strip request headers</span>
            <input
              class="mt-1 w-full py-2 px-3 text-sm font-mono rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
              value={draft.stripRequestHeaders}
              oninput={(e) => draft = { ...draft, stripRequestHeaders: (e.target as HTMLInputElement).value }}
              placeholder="authorization, x-api-key"
            />
          </label>
          <label class="block">
            <span class="text-xs font-medium text-foreground">Strip response headers</span>
            <input
              class="mt-1 w-full py-2 px-3 text-sm font-mono rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
              value={draft.stripResponseHeaders}
              oninput={(e) => draft = { ...draft, stripResponseHeaders: (e.target as HTMLInputElement).value }}
              placeholder="set-cookie"
            />
          </label>
        </div>
      {/if}

      <div class="p-4 grid grid-cols-1 lg:grid-cols-[1fr_auto] gap-3 items-end">
        <label class="block">
          <span class="text-xs font-medium text-foreground">Reason</span>
          <input
            class="mt-1 w-full py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
            value={draft.reason}
            oninput={(e) => draft = { ...draft, reason: (e.target as HTMLInputElement).value }}
            placeholder="Short audit reason"
          />
        </label>
        <div class="flex items-center gap-x-2">
          {#if editingKey}
            <button
              type="button"
              class="p-2 rounded-lg border border-line-2 bg-layer text-foreground hover:bg-layer-hover transition-colors"
              title="Cancel edit"
              onclick={cancelEdit}
            >
              <X size={16} />
            </button>
          {/if}
          <button
            type="button"
            class="py-2 px-4 inline-flex items-center gap-x-1.5 text-sm font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover transition-colors disabled:opacity-50"
            disabled={!draft.name.trim() || !draft.condition.trim()}
            onclick={stageDraft}
          >
            <Plus size={16} />
            Stage rule
          </button>
        </div>
      </div>
    </div>
    {#if stagedMessage}
      <p class="text-xs text-primary mt-2">{stagedMessage}</p>
    {/if}
  </div>

  <div>
    <div class="flex items-center justify-between mb-2">
      <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider">Effective {activeType} rules</h3>
      <span class="text-xs text-muted-foreground-1">{visibleEntries.length} rule{visibleEntries.length === 1 ? '' : 's'}</span>
    </div>
    {#if visibleEntries.length === 0}
      <div class="bg-card border border-card-line rounded-xl p-6 text-center">
        <p class="text-sm text-muted-foreground-1">No named {activeType} rules configured.</p>
      </div>
    {:else}
      <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
        {#each visibleEntries as entry (entry.key)}
          {@const pending = settingsStore.model?.pendingChanges.get(entry.key)}
          <div class="p-4 flex items-start justify-between gap-x-4 {pending === null ? 'opacity-45' : ''}">
            <button type="button" class="min-w-0 text-left flex-1" onclick={() => editRule(entry)}>
              <div class="flex items-center gap-x-2 flex-wrap">
                <span class="text-sm font-mono text-foreground">{entry.name}</span>
                <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground-1">{entry.rule.on}</span>
                <span class="text-[10px] px-1.5 py-0.5 rounded-full {entry.rule.decision === 'block' ? 'bg-destructive/10 text-destructive' : 'bg-primary/10 text-primary'}">{entry.rule.decision}</span>
                {#if pending}
                  <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-warning/10 text-warning">staged</span>
                {:else if pending === null}
                  <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-destructive/10 text-destructive">delete</span>
                {/if}
              </div>
              <p class="text-xs font-mono text-muted-foreground-1 mt-1 break-all">{entry.rule.if}</p>
              {#if entry.rule.reason}
                <p class="text-xs text-muted-foreground-1 mt-1">{entry.rule.reason}</p>
              {/if}
            </button>
            <button
              type="button"
              class="p-1.5 rounded-md text-muted-foreground-1 hover:text-destructive hover:bg-muted-hover transition-colors"
              title="Delete rule"
              onclick={() => deleteRule(entry)}
            >
              <Trash size={16} />
            </button>
          </div>
        {/each}
      </div>
    {/if}
  </div>

  {#if generatedEntries.length > 0}
    <div>
      <div class="flex items-center justify-between mb-2">
        <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider">Generated from settings</h3>
        <button
          type="button"
          class="py-1.5 px-3 inline-flex items-center gap-x-1.5 text-xs font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover transition-colors"
          onclick={stageAllGenerated}
        >
          <Plus size={14} />
          Stage all
        </button>
      </div>
      <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
        {#each generatedEntries.slice(0, 12) as entry (entry.key)}
          <div class="p-4 flex items-start justify-between gap-x-4">
            <div class="min-w-0">
              <div class="flex items-center gap-x-2 flex-wrap">
                <span class="text-sm font-mono text-foreground">{entry.key}</span>
                <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground-1">{entry.rule.decision}</span>
              </div>
              <p class="text-xs font-mono text-muted-foreground-1 mt-1 break-all">{entry.rule.if}</p>
              {#if entry.origin}
                <p class="text-xs text-muted-foreground-1 mt-1">{entry.origin}</p>
              {/if}
            </div>
            <button
              type="button"
              class="p-1.5 rounded-md text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover transition-colors"
              title="Stage generated rule"
              onclick={() => stageGenerated(entry)}
            >
              <Plus size={16} />
            </button>
          </div>
        {/each}
      </div>
    </div>
  {/if}
</div>
