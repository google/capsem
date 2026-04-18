<script lang="ts">
  import { onMount } from 'svelte';
  import * as api from '../../api';
  import type { SecurityPreset } from '../../types/settings';
  import { themeStore, PRELINE_THEMES } from '../../stores/theme.svelte.ts';
  import { THEME_FAMILIES } from '../../terminal/themes';

  // -- Security presets --
  let presets = $state<SecurityPreset[]>([]);
  let selectedPreset = $state<string>('medium');

  // -- VM defaults --
  let cpuCores = $state(4);
  let ramGb = $state(4);
  let maxVms = $state(10);

  onMount(async () => {
    try {
      presets = await api.getPresets();
    } catch {
      // Presets unavailable
    }
    // Load current VM resource settings
    try {
      const settings = await api.getSettings();
      const findLeaf = (id: string): unknown => {
        const walk = (nodes: typeof settings.tree): unknown => {
          for (const n of nodes) {
            if (n.kind === 'leaf' && n.id === id) return n.effective_value;
            if (n.kind === 'group' || (n.kind !== 'leaf' && 'children' in n)) {
              const found = walk((n as { children: typeof settings.tree }).children ?? []);
              if (found !== undefined) return found;
            }
          }
          return undefined;
        };
        return walk(settings.tree);
      };
      const cpu = findLeaf('vm.resources.cpu_count');
      if (typeof cpu === 'number') cpuCores = cpu;
      const ram = findLeaf('vm.resources.ram_gb');
      if (typeof ram === 'number') ramGb = ram;
      const vms = findLeaf('vm.resources.max_concurrent_vms');
      if (typeof vms === 'number') maxVms = vms;
    } catch { /* */ }
  });

  async function onPresetChange() {
    if (selectedPreset) {
      try { await api.applyPreset(selectedPreset); } catch { /* */ }
    }
  }

  async function saveVmDefaults() {
    try {
      await api.saveSettings({
        'vm.resources.cpu_count': cpuCores,
        'vm.resources.ram_gb': ramGb,
        'vm.resources.max_concurrent_vms': maxVms,
      });
    } catch { /* */ }
  }
</script>

<div class="space-y-5">
  <div>
    <h2 class="text-xl font-medium text-foreground">Preferences</h2>
    <p class="mt-1 text-sm text-muted-foreground-1">
      Customize your experience. All settings can be changed later.
    </p>
  </div>

  <!-- Security preset (compact) -->
  <div class="bg-card border border-card-line rounded-xl p-4">
    <div class="flex items-center justify-between">
      <div>
        <span class="text-sm font-medium text-foreground">Security Preset</span>
        <p class="text-xs text-muted-foreground mt-0.5">Controls network access and sandbox policies.</p>
      </div>
      <select
        class="py-1.5 px-3 text-sm border border-line-2 rounded-lg bg-layer text-foreground focus:border-primary focus:ring-1 focus:ring-primary outline-none"
        bind:value={selectedPreset}
        onchange={onPresetChange}
      >
        {#each presets as preset}
          <option value={preset.id}>{preset.name}</option>
        {/each}
      </select>
    </div>
  </div>

  <!-- Appearance -->
  <div class="bg-card border border-card-line rounded-xl p-4 space-y-4">
    <h3 class="text-sm font-medium text-foreground">Appearance</h3>

    <!-- Dark mode -->
    <div class="flex items-center justify-between">
      <span class="text-sm text-muted-foreground-1">Dark mode</span>
      <div class="flex gap-1">
        {#each ['auto', 'light', 'dark'] as mode}
          <button
            type="button"
            class="py-1 px-2.5 text-xs rounded-md transition-colors"
            class:bg-primary={themeStore.modePref === mode}
            class:text-primary-foreground={themeStore.modePref === mode}
            class:bg-layer={themeStore.modePref !== mode}
            class:text-muted-foreground-1={themeStore.modePref !== mode}
            onclick={() => themeStore.setMode(mode as 'auto' | 'light' | 'dark')}
          >
            {mode.charAt(0).toUpperCase() + mode.slice(1)}
          </button>
        {/each}
      </div>
    </div>

    <!-- UI theme -->
    <div class="flex items-center justify-between">
      <span class="text-sm text-muted-foreground-1">Accent theme</span>
      <div class="flex gap-1.5">
        {#each PRELINE_THEMES as t}
          <button
            type="button"
            class="size-5 rounded-full border-2 transition-colors"
            class:border-foreground={themeStore.prelineTheme === t.value}
            class:border-transparent={themeStore.prelineTheme !== t.value}
            style="background-color: {t.color}"
            title={t.label}
            onclick={() => themeStore.setPrelineTheme(t.value)}
          ></button>
        {/each}
      </div>
    </div>

    <!-- Terminal theme -->
    <div class="flex items-center justify-between">
      <span class="text-sm text-muted-foreground-1">Terminal theme</span>
      <select
        class="py-1.5 px-3 text-sm border border-line-2 rounded-lg bg-layer text-foreground focus:border-primary focus:ring-1 focus:ring-primary outline-none"
        value={themeStore.terminalTheme}
        onchange={(e) => themeStore.setTerminalTheme((e.target as HTMLSelectElement).value)}
      >
        {#each THEME_FAMILIES as family}
          <option value={family.name}>{family.label}</option>
        {/each}
      </select>
    </div>
  </div>

  <!-- VM defaults -->
  <div class="bg-card border border-card-line rounded-xl p-4 space-y-4">
    <h3 class="text-sm font-medium text-foreground">VM Defaults</h3>

    <div class="flex items-center justify-between">
      <span class="text-sm text-muted-foreground-1">CPU cores</span>
      <select
        class="py-1.5 px-3 text-sm border border-line-2 rounded-lg bg-layer text-foreground focus:border-primary focus:ring-1 focus:ring-primary outline-none"
        bind:value={cpuCores}
        onchange={saveVmDefaults}
      >
        {#each [1, 2, 4, 6, 8] as n}
          <option value={n}>{n}</option>
        {/each}
      </select>
    </div>

    <div class="flex items-center justify-between">
      <span class="text-sm text-muted-foreground-1">RAM</span>
      <select
        class="py-1.5 px-3 text-sm border border-line-2 rounded-lg bg-layer text-foreground focus:border-primary focus:ring-1 focus:ring-primary outline-none"
        bind:value={ramGb}
        onchange={saveVmDefaults}
      >
        {#each [1, 2, 4, 8, 16] as n}
          <option value={n}>{n} GB</option>
        {/each}
      </select>
    </div>

    <div class="flex items-center justify-between">
      <span class="text-sm text-muted-foreground-1">Max concurrent VMs</span>
      <select
        class="py-1.5 px-3 text-sm border border-line-2 rounded-lg bg-layer text-foreground focus:border-primary focus:ring-1 focus:ring-primary outline-none"
        bind:value={maxVms}
        onchange={saveVmDefaults}
      >
        {#each [1, 2, 5, 10, 20] as n}
          <option value={n}>{n}</option>
        {/each}
      </select>
    </div>
  </div>
</div>
