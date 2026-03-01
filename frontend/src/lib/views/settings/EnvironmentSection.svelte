<script lang="ts">
  import { onMount } from 'svelte';
  import { settingsStore } from '../../stores/settings.svelte';
  import type { ResolvedSetting, SettingValue } from '../../types';

  onMount(() => {
    if (settingsStore.settings.length === 0) settingsStore.load();
  });

  function handleChange(setting: ResolvedSetting, value: SettingValue) {
    settingsStore.update(setting.id, value);
  }

  const envSettings = $derived(settingsStore.byCategory('Guest Environment'));
</script>

<div class="space-y-2">
  {#if envSettings.length === 0}
    <div class="text-sm text-base-content/40 py-8 text-center">No environment settings available</div>
  {:else}
    {#each envSettings as setting}
      <div
        class="flex items-center justify-between rounded-lg border border-base-300 bg-base-100 px-3 py-2"
        class:opacity-40={!setting.enabled}
      >
        <div class="flex-1 min-w-0 mr-4">
          <div class="flex items-center gap-2">
            <span class="text-sm font-medium font-mono">{setting.name}</span>
            {#if setting.corp_locked}
              <span class="badge badge-xs badge-warning">corp</span>
            {/if}
          </div>
          <p class="text-xs text-base-content/50 truncate">{setting.description}</p>
        </div>
        <input
          type="text"
          class="input input-xs input-bordered w-64 font-mono text-xs"
          value={String(setting.effective_value)}
          disabled={setting.corp_locked || !setting.enabled}
          onchange={(e) => handleChange(setting, (e.target as HTMLInputElement).value)}
        />
      </div>
    {/each}
  {/if}
</div>
