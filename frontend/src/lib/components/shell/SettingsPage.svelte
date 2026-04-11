<script lang="ts">
  import { onMount } from 'svelte';
  import { themeStore, PRELINE_THEMES, FONT_SIZES, FONT_FAMILIES, UI_FONT_SIZES } from '../../stores/theme.svelte.ts';
  import { settingsStore } from '../../stores/settings.svelte.ts';
  import { THEME_FAMILIES, getTheme, resolveThemeKey } from '../../terminal/themes';
  import SettingsSection from '../settings/SettingsSection.svelte';
  import McpSection from '../settings/McpSection.svelte';
  import Palette from 'phosphor-svelte/lib/Palette';
  import GearSix from 'phosphor-svelte/lib/GearSix';
  import Brain from 'phosphor-svelte/lib/Brain';
  import GitBranch from 'phosphor-svelte/lib/GitBranch';
  import Shield from 'phosphor-svelte/lib/Shield';
  import Desktop from 'phosphor-svelte/lib/Desktop';
  import Plugs from 'phosphor-svelte/lib/Plugs';
  import Info from 'phosphor-svelte/lib/Info';
  import Sun from 'phosphor-svelte/lib/Sun';
  import Moon from 'phosphor-svelte/lib/Moon';

  // Live preview: resolve current terminal theme to get colors
  let previewTheme = $derived(getTheme(resolveThemeKey(themeStore.terminalTheme, themeStore.mode)));

  // Active section (panel-per-section, not scrollspy)
  let activeSection = $state('appearance');

  // Dynamic sections from settings tree (exclude 'appearance' -- handled by custom UI)
  let dynamicSections = $derived.by(() => {
    const sections = settingsStore.model?.sections ?? [];
    return sections.filter(s => s.key !== 'appearance');
  });

  // Active dynamic group (if sidebar selected a dynamic section)
  let activeDynamicGroup = $derived.by(() => {
    return dynamicSections.find(s => s.key === activeSection);
  });

  // Icon map for dynamic sections
  const SECTION_ICONS: Record<string, any> = {
    app: GearSix,
    ai: Brain,
    repository: GitBranch,
    security: Shield,
    vm: Desktop,
  };

  // Build full nav list: Appearance + dynamic + MCP + About
  let navItems = $derived.by(() => {
    const items: { key: string; label: string; icon: any }[] = [
      { key: 'appearance', label: 'Appearance', icon: Palette },
    ];
    for (const section of dynamicSections) {
      items.push({
        key: section.key,
        label: section.name,
        icon: SECTION_ICONS[section.key] ?? GearSix,
      });
    }
    items.push({ key: 'mcp', label: 'MCP Servers', icon: Plugs });
    items.push({ key: 'about', label: 'About Capsem', icon: Info });
    return items;
  });

  onMount(() => {
    settingsStore.load();
  });

  async function handleSave() {
    await settingsStore.save();
  }

  async function handleDiscard() {
    await settingsStore.discard();
  }
</script>

<div class="flex h-full">
  <!-- Left nav -->
  <aside class="w-56 shrink-0 border-e border-line-2 bg-background overflow-y-auto py-4">
    <h1 class="text-xl font-bold text-foreground px-5 mb-4">Settings</h1>
    <nav class="space-y-0.5 px-3">
      {#each navItems as item (item.key)}
        <button
          type="button"
          class="w-full flex items-center gap-x-3 py-2 px-3 text-sm rounded-lg transition-colors
            {activeSection === item.key
              ? 'bg-muted text-foreground font-medium'
              : 'text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover'}"
          onclick={() => activeSection = item.key}
        >
          <item.icon size={18} />
          {item.label}
        </button>
      {/each}
    </nav>
  </aside>

  <!-- Content (one panel per section) -->
  <main class="flex-1 overflow-y-auto relative">
    <div class="py-6 px-8">

      {#if activeSection === 'appearance'}
        <!-- ===== Appearance (custom, not from backend tree) ===== -->
        <h2 class="text-xl font-bold text-foreground mb-6">Appearance</h2>

        <!-- Interface -->
        <h3 class="text-xs font-semibold text-muted-foreground-1 uppercase tracking-wider mb-2">Interface</h3>
        <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider mb-6">
          <!-- Mode -->
          <div class="flex items-center justify-between p-4">
            <div>
              <p class="text-sm font-medium text-foreground">Mode</p>
              <p class="text-xs text-muted-foreground-1 mt-0.5">Light, dark, or follow system preference</p>
            </div>
            <div class="flex items-center gap-x-1">
              <button
                type="button"
                class="py-2 px-3 inline-flex items-center gap-x-1.5 text-sm font-medium rounded-lg border
                  {themeStore.modePref === 'auto'
                    ? 'bg-primary border-primary-line text-primary-foreground'
                    : 'bg-layer border-layer-line text-layer-foreground hover:bg-layer-hover'}"
                onclick={() => themeStore.setMode('auto')}
              >
                <Desktop size={16} />
                Auto
              </button>
              <button
                type="button"
                class="py-2 px-3 inline-flex items-center gap-x-1.5 text-sm font-medium rounded-lg border
                  {themeStore.modePref === 'light'
                    ? 'bg-primary border-primary-line text-primary-foreground'
                    : 'bg-layer border-layer-line text-layer-foreground hover:bg-layer-hover'}"
                onclick={() => themeStore.setMode('light')}
              >
                <Sun size={16} />
                Light
              </button>
              <button
                type="button"
                class="py-2 px-3 inline-flex items-center gap-x-1.5 text-sm font-medium rounded-lg border
                  {themeStore.modePref === 'dark'
                    ? 'bg-primary border-primary-line text-primary-foreground'
                    : 'bg-layer border-layer-line text-layer-foreground hover:bg-layer-hover'}"
                onclick={() => themeStore.setMode('dark')}
              >
                <Moon size={16} />
                Dark
              </button>
            </div>
          </div>

          <!-- UI Accent -->
          <div class="flex items-center justify-between p-4">
            <div>
              <p class="text-sm font-medium text-foreground">Accent</p>
              <p class="text-xs text-muted-foreground-1 mt-0.5">Color accent for the interface chrome</p>
            </div>
            <div class="flex items-center gap-x-2">
              {#each PRELINE_THEMES as theme (theme.value)}
                <button
                  type="button"
                  class="size-6 rounded-full border-2 transition-transform
                    {themeStore.prelineTheme === theme.value
                      ? 'border-foreground scale-110'
                      : 'border-transparent hover:scale-110'}"
                  style="background-color: {theme.color}"
                  title={theme.label}
                  onclick={() => themeStore.setPrelineTheme(theme.value)}
                ></button>
              {/each}
            </div>
          </div>

          <!-- UI Font Size -->
          <div class="flex items-center justify-between p-4">
            <div>
              <p class="text-sm font-medium text-foreground">UI Size</p>
              <p class="text-xs text-muted-foreground-1 mt-0.5">Base font size for the interface</p>
            </div>
            <select
              class="py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
              value={themeStore.uiFontSize}
              onchange={(e) => themeStore.setUiFontSize(Number((e.target as HTMLSelectElement).value))}
            >
              {#each UI_FONT_SIZES as size (size)}
                <option value={size}>{size}px</option>
              {/each}
            </select>
          </div>
        </div>

        <!-- Terminal -->
        <h3 class="text-xs font-semibold text-muted-foreground-1 uppercase tracking-wider mb-2">Terminal</h3>
        <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider mb-6">
          <!-- Live preview -->
          <div class="p-4">
            <div
              class="rounded-lg overflow-hidden border border-line-2"
              style="background-color: {previewTheme.background}; font-family: {themeStore.fontFamily}; font-size: {themeStore.fontSize}px; line-height: 1.3;"
            >
              <div class="px-3 py-2.5 select-none">
                <span style="color: {previewTheme.blue}; font-weight: bold;">capsem</span><span style="color: {previewTheme.foreground};">:</span><span style="color: {previewTheme.cyan};">~/project</span><span style="color: {previewTheme.foreground};">$ </span><span style="color: {previewTheme.foreground};">ls -la</span>
                <br>
                <span style="color: {previewTheme.green};">drwxr-xr-x</span><span style="color: {previewTheme.foreground};">  5 user user 160 Apr 10 </span><span style="color: {previewTheme.blue};">src/</span>
                <br>
                <span style="color: {previewTheme.foreground};">-rw-r--r--  1 user user  42 Apr 10 </span><span style="color: {previewTheme.foreground};">Cargo.toml</span>
                <br>
                <span style="color: {previewTheme.yellow};">-rwxr-xr-x</span><span style="color: {previewTheme.foreground};">  1 user user 8.2K Apr 10 </span><span style="color: {previewTheme.green};">build.sh</span>
                <br>
                <span style="color: {previewTheme.red};">error</span><span style="color: {previewTheme.foreground};">: </span><span style="color: {previewTheme.magenta};">permission denied</span>
                <br>
                <span style="color: {previewTheme.blue}; font-weight: bold;">capsem</span><span style="color: {previewTheme.foreground};">:</span><span style="color: {previewTheme.cyan};">~/project</span><span style="color: {previewTheme.foreground};">$ </span><span style="color: {previewTheme.cursor}; opacity: 0.7;">|</span>
              </div>
            </div>
          </div>

          <!-- Terminal theme grid -->
          <div class="p-4">
            <div class="flex items-center justify-between mb-3">
              <div>
                <p class="text-sm font-medium text-foreground">Color Scheme</p>
                <p class="text-xs text-muted-foreground-1 mt-0.5">Auto-adapts to light and dark mode</p>
              </div>
            </div>
            <div class="grid grid-cols-3 gap-2">
              {#each THEME_FAMILIES as family (family.name)}
                <button
                  type="button"
                  class="flex items-center gap-x-2.5 py-2 px-3 rounded-lg border text-sm transition-colors
                    {themeStore.terminalTheme === family.name
                      ? 'border-primary bg-primary/5 text-foreground font-medium'
                      : 'border-line-2 bg-layer text-muted-foreground-1 hover:text-foreground hover:border-line-3'}"
                  onclick={() => themeStore.setTerminalTheme(family.name)}
                >
                  <span class="flex gap-px shrink-0">
                    {#each family.colors as color}
                      <span class="w-2.5 h-4 first:rounded-l last:rounded-r" style="background-color: {color}"></span>
                    {/each}
                  </span>
                  <span class="truncate">{family.label}</span>
                </button>
              {/each}
            </div>
          </div>

          <!-- Font family -->
          <div class="flex items-center justify-between p-4">
            <div>
              <p class="text-sm font-medium text-foreground">Font</p>
              <p class="text-xs text-muted-foreground-1 mt-0.5">Monospace font for terminal emulators</p>
            </div>
            <select
              class="py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary min-w-48"
              value={themeStore.fontFamily}
              onchange={(e) => themeStore.setFontFamily((e.target as HTMLSelectElement).value)}
            >
              {#each FONT_FAMILIES as font (font.value)}
                <option value={font.value} style="font-family: {font.value}">{font.label}</option>
              {/each}
            </select>
          </div>

          <!-- Font size -->
          <div class="flex items-center justify-between p-4">
            <div>
              <p class="text-sm font-medium text-foreground">Font Size</p>
              <p class="text-xs text-muted-foreground-1 mt-0.5">Font size in pixels</p>
            </div>
            <select
              class="py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary"
              value={themeStore.fontSize}
              onchange={(e) => themeStore.setFontSize(Number((e.target as HTMLSelectElement).value))}
            >
              {#each FONT_SIZES as size (size)}
                <option value={size}>{size}px</option>
              {/each}
            </select>
          </div>
        </div>

      {:else if activeSection === 'mcp'}
        <!-- ===== MCP ===== -->
        <McpSection />

      {:else if activeSection === 'about'}
        <!-- ===== About ===== -->
        <h2 class="text-xl font-bold text-foreground mb-6">About Capsem</h2>
        <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
          <div class="flex items-center justify-between p-4">
            <p class="text-sm text-foreground">Version</p>
            <p class="text-sm text-muted-foreground-1">0.1.0-dev</p>
          </div>
          <div class="flex items-center justify-between p-4">
            <p class="text-sm text-foreground">Runtime</p>
            <p class="text-sm text-muted-foreground-1">Apple Virtualization.framework</p>
          </div>
          <div class="flex items-center justify-between p-4">
            <p class="text-sm text-foreground">Kernel</p>
            <p class="text-sm text-muted-foreground-1">6.12-capsem</p>
          </div>
        </div>

      {:else if activeDynamicGroup}
        <!-- ===== Dynamic section from settings tree ===== -->
        <SettingsSection group={activeDynamicGroup} />
      {/if}
    </div>

    <!-- Dirty bar (sticky at bottom) -->
    {#if settingsStore.isDirty}
      <div class="sticky bottom-0 bg-background border-t border-line-2 px-6 py-3 flex items-center justify-end gap-x-3 shadow-lg">
        <span class="text-xs text-muted-foreground-1 mr-auto">
          {settingsStore.model?.pendingChanges.size ?? 0} unsaved change{(settingsStore.model?.pendingChanges.size ?? 0) === 1 ? '' : 's'}
        </span>
        <button
          type="button"
          class="py-2 px-4 text-sm font-medium rounded-lg border border-line-2 bg-layer text-foreground hover:bg-layer-hover transition-colors"
          onclick={handleDiscard}
        >
          Discard
        </button>
        <button
          type="button"
          class="py-2 px-4 text-sm font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover transition-colors"
          onclick={handleSave}
        >
          Save
        </button>
      </div>
    {/if}
  </main>
</div>
