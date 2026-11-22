<script lang="ts">
  import { untrack } from 'svelte';
  import type { McpCatalogDetailResponse } from '@capsem/sdk';
  import type { McpCatalogInput } from '../../models/environment-editor';
  import EnvironmentMcpLaunch from './EnvironmentMcpLaunch.svelte';

  let { detail, initial, selected, onstage }: {
    detail: McpCatalogDetailResponse;
    initial?: McpCatalogInput;
    selected: boolean;
    onstage: (input: McpCatalogInput) => void;
  } = $props();
  const inputClass = 'w-full rounded-md border border-line-2 bg-background-1 px-2.5 py-1.5 text-sm';
  const uuidPattern = '[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}';
  let values = $state(untrack(() => Object.entries(
    detail.definition.kind === 'remote' ? detail.definition.headers ?? {} : detail.definition.variables ?? {},
  ).map(([name, requirement]) => {
    const saved = initial?.values && Object.hasOwn(initial.values, name) ? initial.values[name] : undefined;
    return { name, ...requirement, value: saved ?? '', savedEmpty: saved === '' };
  })));
  let binding = $state(untrack(() => initial?.binding_id ?? ''));
  let secrets = $state(untrack(() => {
    if (detail.definition.kind !== 'sandbox') return [];
    const previous = Object.entries(initial?.secrets ?? {});
    const aliases = new Set(previous.map(([alias]) => alias));
    let index = 1;
    return Object.entries(detail.definition.secret_variables ?? {}).map(([name, requirement]) => {
      const retained = previous.find(([, secret]) => secret.env_name === name);
      if (retained) return { name, alias: retained[0], value: retained[1].binding_id, ...requirement };
      while (aliases.has(`binding-${index}`)) index += 1;
      return { name, alias: `binding-${index++}`, value: '', ...requirement };
    });
  }));

  function submit(event: SubmitEvent) {
    event.preventDefault();
    onstage({
      values: Object.fromEntries(values.filter(field => field.value !== '' || field.savedEmpty).map(field => [field.name, field.value])),
      ...(binding ? { binding_id: binding } : {}),
      secrets: Object.fromEntries(secrets.filter(field => field.value).map(field => [
        field.alias, { binding_id: field.value, env_name: field.name },
      ])),
    });
  }
</script>

<EnvironmentMcpLaunch definition={detail.definition} />
{#if detail.published_tools?.length}
  <details class="space-y-2 text-xs">
    <summary class="cursor-pointer font-medium">Published tool index</summary>
    <ul class="space-y-2">
      {#each detail.published_tools as tool (tool.name)}
        <li><span class="break-words font-mono">{tool.name}</span>{#if tool.description}<p class="whitespace-pre-wrap break-words">{tool.description}</p>{/if}</li>
      {/each}
    </ul>
  </details>
{/if}
<form class="space-y-3" onsubmit={submit}>
  {#each values as field (field.name)}
    <label class="block min-w-0 space-y-1">
      <span class="block break-words text-xs font-medium">{field.name}{field.required ? ' (required)' : ''}</span>
      <span class="block break-words text-xs text-muted-foreground-1">{field.description}</span>
      {#if detail.definition.kind === 'sandbox'}
        <textarea rows="2" class={inputClass} bind:value={field.value} required={field.required && !field.savedEmpty}></textarea>
      {:else}
        <input type="text" class={inputClass} bind:value={field.value} required={field.required && !field.savedEmpty} />
      {/if}
    </label>
  {/each}
  {#if detail.definition.kind === 'remote' && detail.definition.secret_binding}
    <label class="block space-y-1">
      <span class="block text-xs font-medium">Credential binding ID{detail.definition.secret_binding.required ? ' (required)' : ''}</span>
      <span class="block text-xs text-muted-foreground-1">{detail.definition.secret_binding.description}</span>
      <input type="text" class={`${inputClass} font-mono`} bind:value={binding} required={detail.definition.secret_binding.required}
        pattern={uuidPattern} maxlength="36" autocomplete="off" spellcheck="false" />
    </label>
  {/if}
  {#each secrets as field (field.name)}
    <label class="block min-w-0 space-y-1">
      <span class="block break-words text-xs font-medium">{field.name} binding ID{field.required ? ' (required)' : ''}</span>
      <span class="block break-words text-xs text-muted-foreground-1">{field.description}</span>
      <input type="text" class={`${inputClass} font-mono`} bind:value={field.value} required={field.required}
        pattern={uuidPattern} maxlength="36" autocomplete="off" spellcheck="false" />
    </label>
  {/each}
  <button type="submit" class="rounded-md bg-primary px-2.5 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary-hover">
    {selected ? 'Update MCP server in draft' : 'Add MCP server to draft'}
  </button>
</form>
