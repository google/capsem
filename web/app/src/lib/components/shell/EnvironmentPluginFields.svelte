<script lang="ts">
  import { onMount } from 'svelte';
  import type { EnvironmentPluginEntry, EnvironmentPluginListResponse, EnvironmentRevisionReference, EnvironmentPluginField } from '@capsem/sdk';
  import { listEnvironmentPlugins } from '../../api';
  import { sessionActionError } from '../../models/session-action-error';
  import type { SettingsLeaf, SettingValue } from '../../types/settings';
  import SelectControl from '../settings/widgets/SelectControl.svelte';
  import ToggleControl from '../settings/widgets/ToggleControl.svelte';

  let { reference, value = $bindable() }: { reference: EnvironmentRevisionReference | null; value: string } = $props();
  let result = $state<EnvironmentPluginListResponse | null>(null);
  let error = $state<string | null>(null);
  let reader: AbortController | undefined;
  let draft = $derived(JSON.parse(value) as Record<string, unknown>);

  async function load() {
    reader?.abort();
    const controller = new AbortController();
    reader = controller;
    error = null;
    try {
      const response = await listEnvironmentPlugins(reference, controller.signal);
      if (!controller.signal.aborted) result = response;
    } catch (reason) {
      if (!controller.signal.aborted) error = sessionActionError(reason);
    }
  }

  onMount(() => { void load(); return () => reader?.abort(); });

  function record(value: unknown): Record<string, unknown> {
    return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
  }

  function read(entry: EnvironmentPluginEntry, path: string): unknown {
    if (entry.locked) return entry.effective[path];
    return path.split('.').reduce((parent, step) => record(parent)[step], draft[entry.descriptor.id]);
  }

  function write(current: unknown, [key, ...children]: string[], value: SettingValue): unknown {
    if (!key) return value;
    const parent = record(current);
    return { ...parent, [key]: write(parent[key], children, value) };
  }

  function update(entry: EnvironmentPluginEntry, path: string, next: SettingValue) {
    if (entry.locked) return;
    const id = entry.descriptor.id;
    value = JSON.stringify({ ...draft, [id]: write(draft[id], path.split('.'), next) }, null, 2);
  }

  function reset(entry: EnvironmentPluginEntry) {
    if (entry.locked) return;
    const next = { ...draft };
    delete next[entry.descriptor.id];
    value = JSON.stringify(next, null, 2);
  }

  function enabled(entry: EnvironmentPluginEntry, field: EnvironmentPluginField): boolean {
    return field.dependencies.every(({ field, not_equals }) => {
      const selected = read(entry, field) ?? entry.descriptor.fields.find(candidate => candidate.id === field)?.default_value;
      return selected !== not_equals;
    });
  }

  function control(entry: EnvironmentPluginEntry, setting: EnvironmentPluginField): SettingsLeaf | null {
    const fallback = setting.default_value;
    const current = read(entry, setting.id) ?? fallback;
    if ((typeof fallback !== 'boolean' && typeof fallback !== 'string') ||
        (typeof current !== 'boolean' && typeof current !== 'string')) return null;
    return {
      kind: 'leaf', id: setting.id, name: setting.name, category: setting.category, description: setting.description,
      setting_type: typeof fallback === 'boolean' ? 'bool' : 'text', default_value: fallback,
      effective_value: current, source: entry.locked ? 'corp' : 'user', modified: null,
      corp_locked: entry.locked, enabled_by: null, enabled: enabled(entry, setting),
      metadata: { choices: setting.choices, domains: [], min: null, max: null, rules: {} },
    };
  }

  function label(value: unknown): string {
    return typeof value === 'boolean' ? value ? 'On' : 'Off' : String(value);
  }
</script>

<details class="rounded-lg border border-line-2">
  <summary class="cursor-pointer px-3 py-2 text-sm font-medium">Plugins</summary>
  <div class="space-y-2 p-3 pt-0">
    {#if error}
      <p role="alert" class="text-destructive-text">{error}</p>
      <button type="button" class="text-primary hover:underline" onclick={load}>Retry plugin settings</button>
    {:else if !result}
      <p role="status" class="text-muted-foreground-1">Loading plugin settings…</p>
    {:else}
      {#each result.plugins as entry (entry.descriptor.id)}
        <details class="rounded-md border border-line-2">
          <summary class="cursor-pointer px-3 py-2">
            <span class="font-medium">{entry.descriptor.name}</span>
            {#if entry.locked}<span class="ms-2 text-xs text-muted-foreground-1">Corporate policy · Locked</span>
            {:else if entry.descriptor.required}<span class="ms-2 text-xs text-muted-foreground-1">Required</span>
            {:else if entry.descriptor.diagnostic}<span class="ms-2 text-xs text-muted-foreground-1">Diagnostic</span>{/if}
          </summary>
          <div class="space-y-3 px-3 pb-3">
            <p class="text-muted-foreground-1">{entry.descriptor.description}</p>
            {#if entry.descriptor.limitations.length}
              <ul class="list-disc space-y-1 ps-4 text-xs text-muted-foreground-1">
                {#each entry.descriptor.limitations as limitation (limitation)}<li>{limitation}</li>{/each}
              </ul>
            {/if}
            <div class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground-1">
              <span>{entry.source === 'corporate' ? 'Corporate effective modes' : entry.source === 'environment' ? 'Saved effective modes' : 'Built-in effective modes'}</span>
              {#each entry.directions as direction (direction.direction)}
                <span>{direction.direction === 'inbound' ? 'Inbound' : 'Outbound'} {direction.mode}</span>
              {/each}
            </div>
            {#each entry.descriptor.groups as group (group.path)}
              <fieldset class="min-w-0 rounded-md border border-line-2 px-3 pb-1">
                <legend class="px-1 text-xs font-medium">{group.name}</legend>
                {#each entry.descriptor.fields.filter(({ category }) => category === group.name) as field (field.id)}
                  {@const leaf = control(entry, field)}
                  {#if leaf && field.widget === 'toggle'}
                    <ToggleControl {leaf} disabled={entry.locked || !leaf.enabled} onchange={(next) => update(entry, field.id, next)} />
                  {:else if leaf && field.widget === 'select'}
                    <SelectControl {leaf} disabled={entry.locked || !leaf.enabled} onchange={(next) => update(entry, field.id, next)} />
                  {:else}
                    <p class="py-2 text-xs text-muted-foreground-1">{field.name} · {label(leaf?.effective_value ?? read(entry, field.id) ?? field.default_value)}</p>
                  {/if}
                  {#if entry.locked && entry.configured[field.id] !== undefined && entry.configured[field.id] !== entry.effective[field.id]}
                    <p class="text-xs text-muted-foreground-1">Saved {field.name.toLowerCase()} {label(entry.configured[field.id])}</p>
                  {/if}
                {/each}
              </fieldset>
            {/each}
            {#if !entry.locked && draft[entry.descriptor.id] != null}
              <button type="button" class="text-xs text-primary hover:underline" onclick={() => reset(entry)}>Use built-in defaults</button>
            {/if}
          </div>
        </details>
      {/each}
    {/if}
  </div>
</details>
