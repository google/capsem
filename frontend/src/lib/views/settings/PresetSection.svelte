<script lang="ts">
  import { settingsStore } from '../../stores/settings.svelte';

  const presets = $derived(settingsStore.presets);
  const applying = $derived(settingsStore.applyingPreset);
  const activeId = $derived(settingsStore.activePresetId);

  function onChange(e: Event) {
    const select = e.target as HTMLSelectElement;
    const val = select.value;
    if (!val || val === 'personalized') {
      select.value = activeId ?? 'personalized';
      return;
    }
    const preset = presets.find(p => p.id === val);
    if (!preset) return;

    const confirmed = confirm(`Apply "${preset.name}"?\n\nThis will overwrite your current security settings.`);
    if (confirmed) {
      settingsStore.applySecurityPreset(val);
    } else {
      select.value = activeId ?? 'personalized';
    }
  }
</script>

{#if presets.length > 0}
  <div class="py-2">
    {#if applying}
      <div class="flex items-center gap-2">
        <span class="loading loading-spinner loading-xs"></span>
        <span class="text-xs text-base-content/50">Applying...</span>
      </div>
    {:else}
      <select
        class="select select-sm select-bordered w-48 text-xs"
        value={activeId ?? 'personalized'}
        onchange={onChange}
      >
        {#if activeId === null}
          <option value="personalized" disabled>Personalized</option>
        {/if}
        {#each presets as preset}
          <option value={preset.id}>{preset.name}</option>
        {/each}
      </select>
    {/if}
  </div>
{/if}
