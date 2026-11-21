<script lang="ts">
  import type { EnvironmentRevisionReference } from '@capsem/sdk';
  import { environmentSectionKeys, environmentSections, type EnvironmentFields } from '../../models/environment-editor';
  import EnvironmentPluginFields from './EnvironmentPluginFields.svelte';
  let { fields = $bindable(), pin, reference }: { fields: EnvironmentFields; pin?: string; reference: EnvironmentRevisionReference | null } = $props();
  const id = $props.id();
  const input = 'w-full rounded-lg border border-line-2 bg-background-1 px-3 py-2 text-sm text-foreground outline-hidden focus:border-primary';
</script>

<div class="space-y-4">
  <div class="space-y-1">
    <label for={`${id}-name`} class="text-sm font-medium">Environment name</label>
    <input id={`${id}-name`} class={input} bind:value={fields.name} maxlength="64" autocomplete="off" />
  </div>
  <div class="space-y-1">
    <label for={`${id}-description`} class="text-sm font-medium">Description</label>
    <textarea id={`${id}-description`} class={input} bind:value={fields.description} rows="2"></textarea>
  </div>
  <div class="grid gap-3 sm:grid-cols-[minmax(0,1fr)_8rem]">
    <div class="space-y-1">
      <label for={`${id}-image`} class="text-sm font-medium">Image reference</label>
      <input id={`${id}-image`} class={input} bind:value={fields.image} placeholder="Plain VM" autocomplete="off" />
    </div>
    <div class="space-y-1">
      <label for={`${id}-policy`} class="text-sm font-medium">Image updates</label>
      <select id={`${id}-policy`} class={input} bind:value={fields.imageUpdates} disabled={!fields.image.trim()}>
        <option value="manual">Manual</option>
        <option value="ask">Ask</option>
        <option value="auto">Automatic</option>
      </select>
    </div>
    {#if pin}<p class="break-all text-xs text-muted-foreground-1 sm:col-span-2">Current image digest {pin}</p>{/if}
  </div>

  <fieldset class="space-y-2 rounded-lg border border-line-2 p-3">
    <legend class="px-1 text-sm font-medium">Startup</legend>
    <label class="flex items-center gap-2 text-sm">
      <input type="checkbox" bind:checked={fields.hasStartup} /> Run a startup script
    </label>
    {#if fields.hasStartup}
      <div class="space-y-1">
        <label for={`${id}-script`} class="text-sm">Startup script</label>
        <textarea id={`${id}-script`} class={`${input} font-mono`} bind:value={fields.script} rows="5" spellcheck="false"></textarea>
      </div>
      <div class="space-y-1">
        <label for={`${id}-timeout`} class="text-sm">Timeout in seconds</label>
        <input id={`${id}-timeout`} type="number" min="1" class={`${input} max-w-36`} bind:value={fields.timeout} />
      </div>
    {/if}
  </fieldset>

  <div class="space-y-2">
    {#each environmentSectionKeys as key (key)}
      {#if key === 'plugins'}
      <EnvironmentPluginFields {reference} bind:value={fields.sections.plugins} />
      {:else}
      <details class="rounded-lg border border-line-2">
        <summary class="cursor-pointer px-3 py-2 text-sm font-medium">{environmentSections[key]}</summary>
        <div class="space-y-1 p-3 pt-0">
          <label for={`${id}-${key}`} class="text-xs text-muted-foreground-1">{environmentSections[key]} (JSON)</label>
          {#if key === 'secrets'}<p class="text-xs text-muted-foreground-1">Use credential binding IDs.</p>{/if}
          <textarea id={`${id}-${key}`} class={`${input} font-mono`} bind:value={fields.sections[key]} rows="5" spellcheck="false"></textarea>
        </div>
      </details>
      {/if}
    {/each}
  </div>
</div>
