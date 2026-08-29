<script lang="ts">
  import type { SettingsLeaf, SettingValue } from '../../../types/settings';

  let { leaf, disabled = false, onchange }: {
    leaf: SettingsLeaf;
    disabled?: boolean;
    onchange: (value: SettingValue) => void;
  } = $props();
</script>

<div class="flex items-start justify-between gap-x-4 py-2">
  <div class="flex-1 min-w-0">
    <div class="flex items-center gap-x-2">
      <span class="text-sm font-medium text-foreground">{leaf.name}</span>
      {#if leaf.corp_locked}
        <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-destructive/10 text-destructive font-medium">corp</span>
      {/if}
    </div>
    {#if leaf.description}
      <p class="text-xs text-muted-foreground-1 mt-0.5">{leaf.description}</p>
    {/if}
  </div>
  <select
    class="py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground
      focus:outline-hidden focus:border-primary w-40 shrink-0
      {disabled ? 'opacity-50 cursor-not-allowed' : ''}"
    value={String(leaf.effective_value)}
    {disabled}
    onchange={(e) => onchange((e.target as HTMLSelectElement).value)}
  >
    {#each leaf.metadata.choices as choice (choice)}
      <option value={choice}>{choice}</option>
    {/each}
  </select>
</div>
