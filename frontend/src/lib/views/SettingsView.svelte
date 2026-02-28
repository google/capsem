<script lang="ts">
  import { onMount } from 'svelte';
  import { settingsStore } from '../stores/settings.svelte';
  import { networkStore } from '../stores/network.svelte';
  import type { ResolvedSetting, SettingValue } from '../types';

  interface ProviderGroup {
    name: string;
    prefix: string;
    toggle: ResolvedSetting;
    core: ResolvedSetting[];     // api_key, domains
    advanced: ResolvedSetting[]; // gemini configs, etc.
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

  // AI Providers get special card layout with toggle + children grouping.
  // Detect provider prefixes dynamically from settings with ".allow" suffix.
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

      // Derive display name from the toggle's name (strip "Allow " prefix).
      const name = s.name.replace(/^Allow\s+/i, '');

      groups.push({ name, prefix, toggle: s, core, advanced });
    }
    return groups;
  }

  // Categories that use the provider card layout (have .allow toggles with children).
  const PROVIDER_CATEGORIES = ['AI Providers'];

  // Whether a category uses the grouped provider card layout.
  function isProviderCategory(category: string): boolean {
    return PROVIDER_CATEGORIES.includes(category);
  }

  let expandedProviders = $state<Record<string, boolean>>({});
  let expandedAdvanced = $state<Record<string, boolean>>({});
  let expandedCategories = $state<Record<string, boolean>>({});

  function toggleProvider(prefix: string) {
    expandedProviders = { ...expandedProviders, [prefix]: !expandedProviders[prefix] };
  }

  function toggleAdvanced(prefix: string) {
    expandedAdvanced = { ...expandedAdvanced, [prefix]: !expandedAdvanced[prefix] };
  }

  const DOMAIN_LIST_IDS = ['network.custom_allow', 'network.custom_block'];

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

  // Domain list helpers.
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

  const sessionStats = $derived(networkStore.stats?.stats);

  function formatTokens(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return `${n}`;
  }

  function formatCost(usd: number): string {
    if (usd === 0) return '$0.00';
    if (usd < 0.01) return `$${usd.toFixed(4)}`;
    return `$${usd.toFixed(2)}`;
  }

  function formatBytes(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)} MB`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)} KB`;
    return `${n} B`;
  }

  let netStatsExpanded = $state(false);
  let statsExpanded = $state(false);

  let fieldErrors = $state<Record<string, string>>({});

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
</script>

<div class="flex h-full flex-col overflow-hidden">
  <div class="flex items-center gap-2 border-b border-base-300 bg-base-200 px-3 py-1.5">
    <span class="text-xs font-semibold">Settings</span>
    {#if settingsStore.loading}
      <span class="loading loading-spinner loading-xs"></span>
    {/if}
  </div>
  <div class="flex-1 overflow-auto p-4">
    <!-- Network Statistics -->
    {#if sessionStats && sessionStats.net_total > 0}
      <div class="mb-4">
        <button
          class="flex items-center gap-2 mb-2 cursor-pointer w-full"
          onclick={() => netStatsExpanded = !netStatsExpanded}
        >
          <span class="text-[10px] text-base-content/40">{netStatsExpanded ? '\u25BC' : '\u25B6'}</span>
          <h3 class="text-sm font-semibold">Network Statistics</h3>
        </button>

        {#if netStatsExpanded}
          <div class="grid grid-cols-4 gap-3">
            <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
              <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Total</div>
              <div class="mt-1 text-xl font-semibold tabular-nums">{sessionStats.net_total}</div>
            </div>
            <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
              <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Allowed</div>
              <div class="mt-1 text-xl font-semibold tabular-nums text-info">{sessionStats.net_allowed}</div>
            </div>
            <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
              <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Denied</div>
              <div class="mt-1 text-xl font-semibold tabular-nums text-secondary">{sessionStats.net_denied}</div>
            </div>
            <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
              <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Traffic</div>
              <div class="mt-1 text-lg font-semibold tabular-nums">{formatBytes(sessionStats.net_bytes_sent + sessionStats.net_bytes_received)}</div>
              <div class="mt-1 flex gap-2 text-[10px] text-base-content/50">
                <span>{formatBytes(sessionStats.net_bytes_sent)} sent</span>
                <span>{formatBytes(sessionStats.net_bytes_received)} recv</span>
              </div>
            </div>
          </div>
        {/if}
      </div>
    {/if}

    {#each settingsStore.categories as category}
      {@const catExpanded = expandedCategories[category] ?? false}
      <div class="mb-4">
        <!-- Category header (click to expand/collapse) -->
        <button
          class="flex items-center gap-2 mb-2 cursor-pointer w-full"
          onclick={() => expandedCategories = { ...expandedCategories, [category]: !catExpanded }}
        >
          <span class="text-[10px] text-base-content/40">{catExpanded ? '\u25BC' : '\u25B6'}</span>
          <h3 class="text-sm font-semibold">{category}</h3>
          <span class="text-[10px] text-base-content/40">({settingsStore.byCategory(category).length})</span>
        </button>

        {#if catExpanded}
          {#if isProviderCategory(category)}
            <!-- Provider card layout -->
            <div class="flex flex-col gap-2">
              {#each groupProviders(settingsStore.byCategory(category)) as provider}
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
            </div>
          {:else}
            <!-- Flat settings list -->
            <div class="flex flex-col gap-2">
              {#each settingsStore.byCategory(category) as setting}
                {#if DOMAIN_LIST_IDS.includes(setting.id)}
                  {@const domains = parseDomainList(setting.effective_value)}
                  <div
                    class="rounded-lg border border-base-300 bg-base-100 px-3 py-2"
                    class:opacity-40={!setting.enabled}
                  >
                    <div class="flex items-center gap-2 mb-1">
                      <span class="text-sm font-medium">{setting.name}</span>
                      {#if setting.corp_locked}
                        <span class="badge badge-xs badge-warning" title="Locked by corporate policy">corp</span>
                      {/if}
                      {#if setting.source !== 'default'}
                        <span class="badge badge-xs badge-ghost">{setting.source}</span>
                      {/if}
                    </div>
                    <p class="text-[11px] text-base-content/50 mb-2">{setting.description}</p>
                    <div class="flex flex-wrap gap-1.5 mb-2 min-h-[28px]">
                      {#each domains as domain}
                        <span class="badge badge-sm gap-1 font-mono {setting.id.includes('block') ? 'badge-secondary' : 'badge-info'}">
                          {domain}
                          {#if !setting.corp_locked && setting.enabled}
                            <button
                              class="cursor-pointer opacity-60 hover:opacity-100"
                              onclick={() => removeDomain(setting, domain)}
                              title="Remove {domain}"
                            >x</button>
                          {/if}
                        </span>
                      {:else}
                        <span class="text-xs text-base-content/30 italic">No domains</span>
                      {/each}
                    </div>
                    {#if !setting.corp_locked && setting.enabled}
                      <div class="flex gap-1.5">
                        <input
                          type="text"
                          class="input input-xs input-bordered flex-1 font-mono text-xs"
                          placeholder="Add domain pattern (e.g. *.example.com)"
                          value={domainInputs[setting.id] ?? ''}
                          oninput={(e) => domainInputs = { ...domainInputs, [setting.id]: (e.target as HTMLInputElement).value }}
                          onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); addDomain(setting); } }}
                        />
                        <button
                          class="btn btn-xs btn-outline"
                          onclick={() => addDomain(setting)}
                        >Add</button>
                      </div>
                    {/if}
                  </div>
                {:else}
                  <div
                    class="flex items-center justify-between rounded-lg border border-base-300 bg-base-100 px-3 py-2"
                    class:opacity-40={!setting.enabled}
                  >
                    <div class="flex-1 min-w-0 mr-4">
                      <div class="flex items-center gap-2">
                        <span class="text-sm font-medium">{setting.name}</span>
                        {#if setting.corp_locked}
                          <span class="badge badge-xs badge-warning" title="Locked by corporate policy">corp</span>
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
                      {:else if setting.metadata.choices.length > 0}
                        <select
                          class="select select-xs select-bordered font-mono"
                          disabled={setting.corp_locked || !setting.enabled}
                          value={String(setting.effective_value)}
                          onchange={(e) => handleChange(setting, (e.target as HTMLSelectElement).value)}
                        >
                          {#each setting.metadata.choices as choice}
                            <option value={choice}>{choice}</option>
                          {/each}
                        </select>
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
                          type={inputType(setting.setting_type)}
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
          {/if}
        {/if}
      </div>
    {/each}
    {#if settingsStore.error}
      <div class="flex items-center justify-center h-32 text-error text-sm">
        Failed to load settings: {settingsStore.error}
      </div>
    {:else if settingsStore.settings.length === 0 && !settingsStore.loading}
      <div class="flex items-center justify-center h-32 text-base-content/40 text-sm">
        No settings available
      </div>
    {/if}

    <!-- Session Statistics -->
    {#if sessionStats && (sessionStats.model_call_count > 0 || sessionStats.net_total > 0)}
      <div class="mb-4 mt-2">
        <button
          class="flex items-center gap-2 mb-2 cursor-pointer w-full"
          onclick={() => statsExpanded = !statsExpanded}
        >
          <span class="text-[10px] text-base-content/40">{statsExpanded ? '\u25BC' : '\u25B6'}</span>
          <h3 class="text-sm font-semibold">Session Statistics</h3>
        </button>

        {#if statsExpanded}
          <div class="grid grid-cols-4 gap-3">
            <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
              <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Generations</div>
              <div class="mt-1 text-xl font-semibold tabular-nums">{sessionStats.model_call_count}</div>
              <div class="mt-1 text-[10px] text-base-content/50">{sessionStats.total_tool_calls} tool calls</div>
            </div>
            <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
              <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Tokens</div>
              <div class="mt-1 text-xl font-semibold tabular-nums">{formatTokens(sessionStats.total_input_tokens + sessionStats.total_output_tokens)}</div>
              <div class="mt-1 flex gap-2 text-[10px] text-base-content/50">
                <span>{formatTokens(sessionStats.total_input_tokens)} in</span>
                <span>{formatTokens(sessionStats.total_output_tokens)} out</span>
              </div>
            </div>
            <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
              <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">Cost</div>
              <div class="mt-1 text-xl font-semibold tabular-nums text-info">{formatCost(sessionStats.total_estimated_cost_usd)}</div>
            </div>
            <div class="rounded-lg border border-base-300 bg-base-200/50 p-3">
              <div class="text-[10px] font-semibold text-base-content/50 uppercase tracking-wider">HTTPS</div>
              <div class="mt-1 text-xl font-semibold tabular-nums">{sessionStats.net_total}</div>
              <div class="mt-1 flex gap-2 text-[10px]">
                <span class="text-info">{sessionStats.net_allowed} allowed</span>
                <span class="text-secondary">{sessionStats.net_denied} denied</span>
              </div>
            </div>
          </div>
          {#if sessionStats.net_bytes_sent > 0 || sessionStats.net_bytes_received > 0}
            <div class="mt-2 text-[10px] text-base-content/40 tabular-nums">
              Network: {formatBytes(sessionStats.net_bytes_sent)} sent / {formatBytes(sessionStats.net_bytes_received)} received
            </div>
          {/if}
        {/if}
      </div>
    {/if}
  </div>
</div>
