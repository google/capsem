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

  // Domain list helpers
  let domainInputs = $state<Record<string, string>>({});

  function parseDomainList(value: SettingValue): string[] {
    const str = String(value).trim();
    if (!str) return [];
    return str.split(',').map((d) => d.trim()).filter(Boolean);
  }

  function addDomain(setting: ResolvedSetting) {
    const raw = (domainInputs[setting.id] ?? '').trim();
    if (!raw) return;
    const current = parseDomainList(setting.effective_value);
    const toAdd = raw.split(',').map((d) => d.trim()).filter(Boolean);
    const deduped = [...new Set([...current, ...toAdd])];
    handleChange(setting, deduped.join(','));
    domainInputs = { ...domainInputs, [setting.id]: '' };
  }

  function removeDomain(setting: ResolvedSetting, domain: string) {
    const current = parseDomainList(setting.effective_value);
    const updated = current.filter((d) => d !== domain);
    handleChange(setting, updated.join(','));
  }

  // Find settings
  const defaultAction = $derived(settingsStore.settings.find(s => s.id === 'network.default_action'));
  const searchSettings = $derived(settingsStore.settings.filter(s => s.category === 'Search'));
  const registrySettings = $derived(settingsStore.settings.filter(s => s.category === 'Package Registries'));
  const customAllow = $derived(settingsStore.settings.find(s => s.id === 'network.custom_allow'));
  const customBlock = $derived(settingsStore.settings.find(s => s.id === 'network.custom_block'));
</script>

<div class="space-y-6">
  <!-- Default action -->
  {#if defaultAction}
    <div class="rounded-lg border border-base-300 bg-base-100 px-3 py-3">
      <div class="flex items-center justify-between">
        <div>
          <span class="text-sm font-medium">{defaultAction.name}</span>
          <p class="text-xs text-base-content/50">{defaultAction.description}</p>
        </div>
        <select
          class="select select-sm select-bordered font-mono text-xs"
          disabled={defaultAction.corp_locked}
          value={String(defaultAction.effective_value)}
          onchange={(e) => handleChange(defaultAction, (e.target as HTMLSelectElement).value)}
        >
          {#each defaultAction.metadata.choices as choice}
            <option value={choice}>{choice}</option>
          {/each}
        </select>
      </div>
    </div>
  {/if}

  <!-- Search providers -->
  {#if searchSettings.length > 0}
    <div>
      <h4 class="text-xs font-semibold text-base-content/50 uppercase tracking-wider mb-2">Search Providers</h4>
      <div class="space-y-1.5">
        {#each searchSettings as setting}
          <div class="flex items-center justify-between rounded-lg border border-base-300 bg-base-100 px-3 py-2">
            <div class="flex-1 min-w-0 mr-4">
              <span class="text-sm font-medium">{setting.name.replace(/^Allow\s+/i, '')}</span>
              {#if setting.metadata.domains.length > 0}
                <p class="text-[10px] text-base-content/40 font-mono">{setting.metadata.domains.join(', ')}</p>
              {/if}
            </div>
            <input
              type="checkbox"
              class="toggle toggle-sm toggle-primary"
              checked={setting.effective_value === true}
              disabled={setting.corp_locked}
              onchange={(e) => handleChange(setting, (e.target as HTMLInputElement).checked)}
            />
          </div>
        {/each}
      </div>
    </div>
  {/if}

  <!-- Package registries -->
  {#if registrySettings.length > 0}
    <div>
      <h4 class="text-xs font-semibold text-base-content/50 uppercase tracking-wider mb-2">Package Registries</h4>
      <div class="space-y-1.5">
        {#each registrySettings as setting}
          <div class="flex items-center justify-between rounded-lg border border-base-300 bg-base-100 px-3 py-2">
            <div class="flex-1 min-w-0 mr-4">
              <span class="text-sm font-medium">{setting.name.replace(/^Allow\s+/i, '')}</span>
              {#if setting.metadata.domains.length > 0}
                <p class="text-[10px] text-base-content/40 font-mono">{setting.metadata.domains.join(', ')}</p>
              {/if}
            </div>
            <input
              type="checkbox"
              class="toggle toggle-sm toggle-primary"
              checked={setting.effective_value === true}
              disabled={setting.corp_locked}
              onchange={(e) => handleChange(setting, (e.target as HTMLInputElement).checked)}
            />
          </div>
        {/each}
      </div>
    </div>
  {/if}

  <!-- Custom allow list -->
  {#if customAllow}
    {@const domains = parseDomainList(customAllow.effective_value)}
    <div>
      <h4 class="text-xs font-semibold text-base-content/50 uppercase tracking-wider mb-2">Custom Allow List</h4>
      <div class="rounded-lg border border-base-300 bg-base-100 px-3 py-2">
        <div class="flex flex-wrap gap-1.5 mb-2 min-h-[28px]">
          {#each domains as domain}
            <span class="badge badge-sm gap-1 font-mono badge-info">
              {domain}
              {#if !customAllow.corp_locked && customAllow.enabled}
                <button
                  class="cursor-pointer opacity-60 hover:opacity-100"
                  onclick={() => removeDomain(customAllow, domain)}
                  title="Remove {domain}"
                >x</button>
              {/if}
            </span>
          {:else}
            <span class="text-xs text-base-content/30 italic">No custom domains</span>
          {/each}
        </div>
        {#if !customAllow.corp_locked && customAllow.enabled}
          <div class="flex gap-1.5">
            <input
              type="text"
              class="input input-xs input-bordered flex-1 font-mono text-xs"
              placeholder="Add domain pattern (e.g. *.example.com)"
              value={domainInputs[customAllow.id] ?? ''}
              oninput={(e) => domainInputs = { ...domainInputs, [customAllow.id]: (e.target as HTMLInputElement).value }}
              onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); addDomain(customAllow); } }}
            />
            <button
              class="btn btn-xs btn-outline"
              onclick={() => addDomain(customAllow)}
            >Add</button>
          </div>
        {/if}
      </div>
    </div>
  {/if}

  <!-- Custom block list -->
  {#if customBlock}
    {@const domains = parseDomainList(customBlock.effective_value)}
    <div>
      <h4 class="text-xs font-semibold text-base-content/50 uppercase tracking-wider mb-2">Custom Block List</h4>
      <div class="rounded-lg border border-base-300 bg-base-100 px-3 py-2">
        <div class="flex flex-wrap gap-1.5 mb-2 min-h-[28px]">
          {#each domains as domain}
            <span class="badge badge-sm gap-1 font-mono badge-secondary">
              {domain}
              {#if !customBlock.corp_locked && customBlock.enabled}
                <button
                  class="cursor-pointer opacity-60 hover:opacity-100"
                  onclick={() => removeDomain(customBlock, domain)}
                  title="Remove {domain}"
                >x</button>
              {/if}
            </span>
          {:else}
            <span class="text-xs text-base-content/30 italic">No blocked domains</span>
          {/each}
        </div>
        {#if !customBlock.corp_locked && customBlock.enabled}
          <div class="flex gap-1.5">
            <input
              type="text"
              class="input input-xs input-bordered flex-1 font-mono text-xs"
              placeholder="Add domain pattern to block"
              value={domainInputs[customBlock.id] ?? ''}
              oninput={(e) => domainInputs = { ...domainInputs, [customBlock.id]: (e.target as HTMLInputElement).value }}
              onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); addDomain(customBlock); } }}
            />
            <button
              class="btn btn-xs btn-outline"
              onclick={() => addDomain(customBlock)}
            >Add</button>
          </div>
        {/if}
      </div>
    </div>
  {/if}
</div>
