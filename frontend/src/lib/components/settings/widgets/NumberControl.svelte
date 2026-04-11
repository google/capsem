<script lang="ts">
  import type { SettingsLeaf, SettingValue } from '../../../types/settings';

  let { leaf, disabled = false, onchange }: {
    leaf: SettingsLeaf;
    disabled?: boolean;
    onchange: (value: SettingValue) => void;
  } = $props();

  let hasRange = $derived(leaf.metadata.min !== null || leaf.metadata.max !== null);
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
  <div class="flex flex-col items-end gap-y-0.5 shrink-0">
    <input
      type="number"
      class="py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground font-mono text-right
        focus:outline-hidden focus:border-primary w-28
        {disabled ? 'opacity-50 cursor-not-allowed' : ''}"
      value={Number(leaf.effective_value)}
      min={leaf.metadata.min ?? undefined}
      max={leaf.metadata.max ?? undefined}
      step={leaf.metadata.step ?? undefined}
      {disabled}
      onchange={(e) => onchange(Number((e.target as HTMLInputElement).value))}
    />
    {#if hasRange}
      <span class="text-[10px] text-muted-foreground-1">
        {leaf.metadata.min ?? ''} -- {leaf.metadata.max ?? ''}
      </span>
    {/if}
  </div>
</div>
