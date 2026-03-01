<script lang="ts">
  import { onMount } from 'svelte';
  import { settingsStore } from '../../stores/settings.svelte';
  import type { ResolvedSetting, SettingValue } from '../../types';

  interface ProviderGroup {
    name: string;
    prefix: string;
    toggle: ResolvedSetting;
    core: ResolvedSetting[];
    advanced: ResolvedSetting[];
  }

  onMount(() => {
    settingsStore.load();
  });

  function handleChange(setting: ResolvedSetting, value: SettingValue) {
    settingsStore.update(setting.id, value);
  }

  function inputType(st: string): string {
    switch (st) {
      case 'password':
      case 'apikey':
        return 'password';
      case 'number':
        return 'number';
      case 'url':
        return 'url';
      case 'email':
        return 'email';
      default:
        return 'text';
    }
  }

  function groupProviders(settings: ResolvedSetting[]): ProviderGroup[] {
    const groups: ProviderGroup[] = [];
    const seen = new Set<string>();

    for (const s of settings) {
      if (!s.id.endsWith('.allow')) continue;
      const prefix = s.id.replace('.allow', '');
      if (seen.has(prefix)) continue;
      seen.add(prefix);

      const rest = settings.filter(
        (r) => r.id.startsWith(prefix + '.') && r.id !== s.id,
      );
      const coreSuffixes = ['.api_key', '.domains'];
      const core = rest.filter((r) =>
        coreSuffixes.some((suf) => r.id === prefix + suf),
      );
      const advanced = rest.filter(
        (r) => !coreSuffixes.some((suf) => r.id === prefix + suf),
      );

      const name = s.name.replace(/^Allow\s+/i, '');
      groups.push({ name, prefix, toggle: s, core, advanced });
    }
    return groups;
  }

  function isJsonPath(setting: ResolvedSetting): boolean {
    return (setting.metadata.guest_path ?? '').endsWith('.json');
  }

  function formatJson(value: string): string {
    try {
      return JSON.stringify(JSON.parse(value), null, 2);
    } catch {
      return value;
    }
  }

  let expandedProviders = $state<Record<string, boolean>>({});
  let expandedAdvanced = $state<Record<string, boolean>>({});
  let fieldErrors = $state<Record<string, string>>({});

  function toggleProvider(prefix: string) {
    expandedProviders = { ...expandedProviders, [prefix]: !expandedProviders[prefix] };
  }

  function toggleAdvanced(prefix: string) {
    expandedAdvanced = { ...expandedAdvanced, [prefix]: !expandedAdvanced[prefix] };
  }

  function handleFileChange(setting: ResolvedSetting, raw: string) {
    const trimmed = raw.trim();
    if (trimmed === '') {
      fieldErrors = { ...fieldErrors, [setting.id]: '' };
      handleChange(setting, '');
      return;
    }
    if (isJsonPath(setting)) {
      try {
        JSON.parse(trimmed);
        fieldErrors = { ...fieldErrors, [setting.id]: '' };
        handleChange(setting, JSON.stringify(JSON.parse(trimmed)));
      } catch (e) {
        fieldErrors = { ...fieldErrors, [setting.id]: (e as SyntaxError).message };
      }
      return;
    }
    fieldErrors = { ...fieldErrors, [setting.id]: '' };
    handleChange(setting, trimmed);
  }

  const providerSettings = $derived(settingsStore.byCategory('AI Providers'));
  const providers = $derived(groupProviders(providerSettings));
</script>

<div class="space-y-3">
  {#if settingsStore.loading}
    <div class="flex justify-center py-8"><span class="loading loading-spinner loading-sm"></span></div>
  {:else if providers.length === 0}
    <div class="text-sm text-base-content/40 py-8 text-center">No AI providers configured</div>
  {:else}
    {#each providers as provider}
      {@const enabled = provider.toggle.effective_value === true}
      {@const expanded = expandedProviders[provider.prefix] ?? false}
      <div
        class="rounded-lg border border-base-300 bg-base-100 overflow-hidden"
        class:opacity-50={!enabled}
      >
        <div class="flex items-center justify-between px-3 py-2 bg-base-200/50">
          <button
            class="flex items-center gap-2 cursor-pointer flex-1 min-w-0"
            onclick={() => toggleProvider(provider.prefix)}
          >
            <span class="text-[10px] text-base-content/40">{expanded ? '\u25BC' : '\u25B6'}</span>
            <span class="text-sm font-semibold">{provider.name}</span>
            {#if provider.toggle.corp_locked}
              <span class="badge badge-xs badge-warning" title="Locked by corporate policy">corp</span>
            {/if}
            {#if provider.toggle.source !== 'default'}
              <span class="badge badge-xs badge-ghost">{provider.toggle.source}</span>
            {/if}
          </button>
          <input
            type="checkbox"
            class="toggle toggle-sm toggle-primary"
            checked={enabled}
            disabled={provider.toggle.corp_locked}
            onchange={(e) => handleChange(provider.toggle, (e.target as HTMLInputElement).checked)}
          />
        </div>

        {#if expanded}
          <div class="flex flex-col gap-2 px-3 py-2 border-t border-base-300">
            {#each provider.core as setting}
              <div class="flex flex-col gap-0.5">
                <div class="flex items-center gap-2">
                  <label class="text-xs text-base-content/60" for={setting.id}>{setting.name}</label>
                  {#if setting.corp_locked}
                    <span class="badge badge-xs badge-warning">corp</span>
                  {/if}
                  {#if setting.source !== 'default'}
                    <span class="badge badge-xs badge-ghost">{setting.source}</span>
                  {/if}
                </div>
                <input
                  id={setting.id}
                  type={inputType(setting.setting_type)}
                  class="input input-sm input-bordered w-full font-mono text-xs"
                  value={String(setting.effective_value)}
                  placeholder={setting.description}
                  disabled={setting.corp_locked || !enabled}
                  onchange={(e) => handleChange(setting, (e.target as HTMLInputElement).value)}
                />
              </div>
            {/each}

            {#if provider.advanced.length > 0}
              <button
                class="flex items-center gap-1 text-xs text-base-content/40 hover:text-base-content/60 mt-1 cursor-pointer"
                onclick={() => toggleAdvanced(provider.prefix)}
              >
                <span class="text-[10px]">{expandedAdvanced[provider.prefix] ? '\u25BC' : '\u25B6'}</span>
                <span>{provider.advanced.length} advanced settings</span>
              </button>
              {#if expandedAdvanced[provider.prefix]}
                <div class="flex flex-col gap-2 mt-1 pl-2 border-l-2 border-base-300">
                  {#each provider.advanced as setting}
                    {@const isFile = setting.setting_type === 'file'}
                    {@const isJsonFile = isFile && isJsonPath(setting)}
                    <div class="flex flex-col gap-0.5">
                      <div class="flex items-center gap-2">
                        <label class="text-xs text-base-content/60" for={setting.id}>{setting.name}</label>
                        {#if setting.corp_locked}
                          <span class="badge badge-xs badge-warning">corp</span>
                        {/if}
                        {#if isFile}
                          <span class="badge badge-xs badge-ghost">{isJsonFile ? 'json' : 'file'}</span>
                        {/if}
                      </div>
                      <p class="text-[10px] text-base-content/40">{setting.description}</p>
                      {#if isFile}
                        {@const fieldErr = fieldErrors[setting.id] || ''}
                        <textarea
                          id={setting.id}
                          class="textarea textarea-bordered w-full font-mono text-xs leading-relaxed whitespace-pre {fieldErr ? 'textarea-error' : ''}"
                          rows={Math.min(12, (isJsonFile ? formatJson(String(setting.effective_value)) : String(setting.effective_value)).split('\n').length + 1)}
                          disabled={setting.corp_locked || !enabled}
                          onchange={(e) => handleFileChange(setting, (e.target as HTMLTextAreaElement).value)}
                        >{isJsonFile ? formatJson(String(setting.effective_value)) : String(setting.effective_value)}</textarea>
                        {#if fieldErr}
                          <p class="text-[10px] text-error">Invalid JSON: {fieldErr}</p>
                        {/if}
                      {:else}
                        <input
                          id={setting.id}
                          type={inputType(setting.setting_type)}
                          class="input input-sm input-bordered w-full font-mono text-xs"
                          value={String(setting.effective_value)}
                          disabled={setting.corp_locked || !enabled}
                          onchange={(e) => handleChange(setting, (e.target as HTMLInputElement).value)}
                        />
                      {/if}
                    </div>
                  {/each}
                </div>
              {/if}
            {/if}
          </div>
        {/if}
      </div>
    {/each}
  {/if}
</div>
