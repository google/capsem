<script lang="ts">
  import { onMount } from 'svelte';
  import { settingsStore } from '../../stores/settings.svelte';
  import { themeStore } from '../../stores/theme.svelte';
  import type { ResolvedSetting, SettingValue } from '../../types';

  onMount(() => {
    if (settingsStore.settings.length === 0) settingsStore.load();
  });

  function handleChange(setting: ResolvedSetting, value: SettingValue) {
    settingsStore.update(setting.id, value);
  }

  const appearanceSettings = $derived(settingsStore.byCategory('Appearance'));
</script>

<div class="space-y-4">
  <!-- Theme toggle (controls actual UI theme) -->
  <div class="flex items-center justify-between rounded-lg border border-base-300 bg-base-100 px-3 py-3">
    <div>
      <span class="text-sm font-medium">Theme</span>
      <p class="text-xs text-base-content/50">Switch between light and dark mode</p>
    </div>
    <div class="flex items-center gap-2">
      <span class="text-xs text-base-content/50">{themeStore.theme === 'dark' ? 'Dark' : 'Light'}</span>
      <input
        type="checkbox"
        class="toggle toggle-sm toggle-primary"
        checked={themeStore.theme === 'dark'}
        onchange={() => themeStore.toggle()}
      />
    </div>
  </div>

  <!-- Additional appearance settings from the settings store -->
  {#each appearanceSettings as setting}
    {#if setting.id !== 'appearance.dark_mode'}
      <div
        class="flex items-center justify-between rounded-lg border border-base-300 bg-base-100 px-3 py-2"
        class:opacity-40={!setting.enabled}
      >
        <div class="flex-1 min-w-0 mr-4">
          <span class="text-sm font-medium">{setting.name}</span>
          <p class="text-xs text-base-content/50 truncate">{setting.description}</p>
        </div>
        <div class="flex-shrink-0">
          {#if setting.setting_type === 'number'}
            <input
              type="number"
              class="input input-xs input-bordered w-20 font-mono"
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
    {/if}
  {/each}
</div>
