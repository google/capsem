<script lang="ts">
  import type { SettingsLeaf, SettingValue } from '../../../types/settings';

  let { leaf, disabled = false, onchange }: {
    leaf: SettingsLeaf;
    disabled?: boolean;
    onchange: (value: SettingValue) => void;
  } = $props();

  let checked = $derived(leaf.effective_value === true);
</script>

<div class="flex items-center justify-between py-2">
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
  <button
    type="button"
    class="relative inline-flex shrink-0 h-6 w-11 border-2 border-transparent rounded-full cursor-pointer transition-colors duration-200
      {checked ? 'bg-primary' : 'bg-surface-2'}
      {disabled ? 'opacity-50 cursor-not-allowed' : ''}"
    role="switch"
    aria-checked={checked}
    aria-label="{leaf.name}"
    {disabled}
    onclick={() => { if (!disabled) onchange(!checked); }}
  >
    <span
      class="pointer-events-none inline-block size-5 rounded-full bg-white shadow-sm ring-0 transition-transform duration-200
        {checked ? 'translate-x-5' : 'translate-x-0'}"
    ></span>
  </button>
</div>
