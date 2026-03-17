<script lang="ts">
  import { wizardStore } from '../../stores/wizard.svelte';
  import { settingsStore } from '../../stores/settings.svelte';

  const presets = $derived(settingsStore.presets);
  const activeId = $derived(settingsStore.activePresetId);
  const applying = $derived(settingsStore.applyingPreset);

  async function selectPreset(id: string) {
    await settingsStore.applySecurityPreset(id);
  }

  const presetDetails: Record<string, string[]> = {
    medium: [
      'Read-only web access (GET/HEAD)',
      'All search engines enabled',
      'Write requests blocked',
      'MCP tools run without confirmation',
    ],
    high: [
      'All web access blocked by default',
      'Only Google search allowed',
      'Write requests blocked',
      'MCP tools require confirmation',
    ],
  };
</script>

<div class="space-y-6">
  <div>
    <h2 class="text-2xl font-semibold">Security Preset</h2>
    <p class="text-sm text-base-content/60 mt-1">
      Choose a security level for network access and MCP tool permissions.
    </p>
  </div>

  <div class="grid grid-cols-2 gap-4">
    {#each presets as preset}
      <button
        class="card border p-5 text-left transition-all cursor-pointer
          {activeId === preset.id
            ? 'ring-2 ring-interactive bg-interactive/5 border-interactive/30'
            : 'border-base-300 hover:border-base-content/30'}"
        onclick={() => selectPreset(preset.id)}
        disabled={!!applying}
      >
        <div class="flex items-center gap-2 mb-2">
          <h3 class="font-semibold">{preset.name}</h3>
          {#if applying === preset.id}
            <span class="loading loading-spinner loading-xs"></span>
          {/if}
        </div>
        <p class="text-xs text-base-content/60 mb-3">{preset.description}</p>
        {#if presetDetails[preset.id]}
          <ul class="text-xs text-base-content/50 space-y-1">
            {#each presetDetails[preset.id] as item}
              <li class="flex items-start gap-1.5">
                <span class="text-base-content/30 mt-0.5">--</span>
                <span>{item}</span>
              </li>
            {/each}
          </ul>
        {/if}
      </button>
    {/each}
  </div>

  <p class="text-xs text-base-content/40">
    You can fine-tune individual settings later in Settings.
  </p>

  <!-- Nav -->
  <div class="flex justify-between pt-4">
    <button class="btn btn-ghost btn-sm" onclick={() => wizardStore.back()}>Back</button>
    <div class="flex gap-2">
      <button class="btn btn-ghost btn-sm" onclick={() => wizardStore.next()}>Skip</button>
      <button class="btn bg-interactive text-white btn-sm" onclick={() => wizardStore.next()}>
        Next
      </button>
    </div>
  </div>
</div>
