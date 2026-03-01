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

  const vmSettings = $derived(settingsStore.byCategory('VM'));
</script>

<div class="space-y-2">
  {#if vmSettings.length === 0}
    <div class="text-sm text-base-content/40 py-8 text-center">No resource settings available</div>
  {:else}
    {#each vmSettings as setting}
      <div
        class="flex items-center justify-between rounded-lg border border-base-300 bg-base-100 px-3 py-2"
        class:opacity-40={!setting.enabled}
      >
        <div class="flex-1 min-w-0 mr-4">
          <div class="flex items-center gap-2">
            <span class="text-sm font-medium">{setting.name}</span>
            {#if setting.corp_locked}
              <span class="badge badge-xs badge-warning">corp</span>
            {/if}
            {#if setting.source !== 'default'}
              <span class="badge badge-xs badge-ghost">{setting.source}</span>
            {/if}
          </div>
          <p class="text-xs text-base-content/50 truncate">{setting.description}</p>
        </div>
        <div class="flex-shrink-0">
          {#if setting.setting_type === 'bool'}
            <input
              type="checkbox"
              class="toggle toggle-sm toggle-primary"
              checked={setting.effective_value === true}
              disabled={setting.corp_locked || !setting.enabled}
              onchange={(e) => handleChange(setting, (e.target as HTMLInputElement).checked)}
            />
          {:else if setting.setting_type === 'number'}
            <input
              type="number"
              class="input input-xs input-bordered w-24 font-mono"
              value={setting.effective_value}
              min={setting.metadata.min ?? undefined}
              max={setting.metadata.max ?? undefined}
              disabled={setting.corp_locked || !setting.enabled}
              onchange={(e) => handleChange(setting, Number((e.target as HTMLInputElement).value))}
            />
          {:else}
            <input
              type="text"
              class="input input-xs input-bordered w-48 font-mono"
              value={String(setting.effective_value)}
              disabled={setting.corp_locked || !setting.enabled}
              onchange={(e) => handleChange(setting, (e.target as HTMLInputElement).value)}
            />
          {/if}
        </div>
      </div>
    {/each}
  {/if}
</div>
