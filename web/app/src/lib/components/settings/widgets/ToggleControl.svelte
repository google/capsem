<script lang="ts">
  import type { SettingsLeaf, SettingValue } from '../../../types/settings';
  import Switch from './Switch.svelte';

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
  <Switch label={leaf.name} {checked} {disabled} {onchange}/>
</div>
