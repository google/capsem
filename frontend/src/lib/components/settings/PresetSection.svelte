<script lang="ts">
  import { settingsStore } from '../../stores/settings.svelte.ts';

  let applying = $state(false);

  async function handleChange(e: Event) {
    const id = (e.target as HTMLSelectElement).value;
    if (!id) return;
    const preset = settingsStore.presets.find(p => p.id === id);
    if (!preset) return;
    if (!confirm(`Apply the "${preset.name}" security preset? This will change multiple settings.`)) {
      (e.target as HTMLSelectElement).value = settingsStore.activePresetId ?? '';
      return;
    }
    applying = true;
    try {
      await settingsStore.applySecurityPreset(id);
    } finally {
      applying = false;
    }
  }
</script>

<div class="flex items-center gap-x-3">
  <select
    class="py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground
      focus:outline-hidden focus:border-primary w-48
      {applying ? 'opacity-50 cursor-not-allowed' : ''}"
    value={settingsStore.activePresetId ?? ''}
    disabled={applying}
    onchange={handleChange}
  >
    <option value="">Personalized</option>
    {#each settingsStore.presets as preset (preset.id)}
      <option value={preset.id}>{preset.name}</option>
    {/each}
  </select>
  {#if applying}
    <span class="text-xs text-muted-foreground-1">Applying...</span>
  {/if}
</div>
