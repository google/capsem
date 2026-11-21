<script lang="ts">
  import type { EnvironmentEffectiveAccessRule, EnvironmentEffectiveDetectionRule, EnvironmentRuleSource } from '@capsem/sdk';

  let { rules, value = $bindable() }: {
    rules: (EnvironmentEffectiveAccessRule | EnvironmentEffectiveDetectionRule)[];
    value: string;
  } = $props();

  const sources: Record<EnvironmentRuleSource, string> = {
    builtin: 'Built-in', corporate: 'Corporate · Locked', environment: 'Environment',
    catalog: 'Catalog preset', environment_mcp: 'MCP permission',
  };
  let search = $state('');
  let term = $derived(search.trim().toLowerCase());
  let visible = $derived(rules.filter(rule => [rule.name, rule.id, sources[rule.source]].some(text => text.toLowerCase().includes(term))));
  let draft = $derived(parseDraft(value));

  function object(value: unknown): Record<string, unknown> | null {
    return value != null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null;
  }

  function parseDraft(value: string) {
    try { return object(JSON.parse(value)); } catch { return null; }
  }

  function customRule(key: string | null) {
    const custom = object(draft?.custom);
    return custom && key && Object.hasOwn(custom, key) ? object(custom[key]) : null;
  }

  function enable(key: string, enabled: boolean) {
    const rule = customRule(key);
    if (!rule) return;
    value = JSON.stringify({ ...draft, custom: { ...object(draft?.custom), [key]: { ...rule, enabled } } }, null, 2);
  }
</script>

<div class="space-y-2">
  {#if rules.length}
    <label class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground-1">
      Filter effective rules
      <input type="search" class="min-w-40 flex-1 rounded-md border border-line-2 bg-background-1 px-2 py-1 text-foreground" bind:value={search} />
    </label>
  {/if}
  {#each visible as rule (rule.id)}
    {@const configured = customRule(rule.editable_key)}
    {@const enabled = configured?.enabled !== false}
    <div class="rounded-md border border-line-2 p-2.5" role="group" aria-label={rule.name}>
      <div class="flex flex-wrap items-start justify-between gap-2">
        <div>
          <p class="font-medium">{rule.name}</p>
          <p class="text-xs text-muted-foreground-1">
            {sources[rule.source]} · {#if 'action' in rule}{rule.action} · Priority {rule.priority}{:else}{rule.level} severity{/if}
            {#if !rule.enabled} · Disabled in saved revision{/if}
          </p>
        </div>
        {#if configured && rule.editable_key}
          {@const key = rule.editable_key}
          <label class="flex items-center gap-1.5 text-xs">
            <input type="checkbox" checked={enabled} onchange={event => enable(key, event.currentTarget.checked)} /> Enabled in draft
          </label>
        {/if}
      </div>
      <code class="mt-2 block break-all text-xs">{rule.condition}</code>
    </div>
  {:else}
    <p class="text-xs text-muted-foreground-1">{rules.length ? 'No rules match the filter.' : 'No effective rules in this section.'}</p>
  {/each}
</div>
