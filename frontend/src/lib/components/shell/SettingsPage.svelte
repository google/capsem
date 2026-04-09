<script lang="ts">
  import { themeStore, PRELINE_THEMES, FONT_SIZES, FONT_FAMILIES, UI_FONT_SIZES } from '../../stores/theme.svelte.ts';
  import { THEME_FAMILIES, getTheme, resolveThemeKey } from '../../terminal/themes';
  import Palette from 'phosphor-svelte/lib/Palette';
  import GearSix from 'phosphor-svelte/lib/GearSix';
  import Shield from 'phosphor-svelte/lib/Shield';
  import WifiHigh from 'phosphor-svelte/lib/WifiHigh';
  import HardDrives from 'phosphor-svelte/lib/HardDrives';
  import Terminal from 'phosphor-svelte/lib/Terminal';
  import Info from 'phosphor-svelte/lib/Info';
  import Sun from 'phosphor-svelte/lib/Sun';
  import Moon from 'phosphor-svelte/lib/Moon';
  import Desktop from 'phosphor-svelte/lib/Desktop';

  type Section = 'appearance' | 'general' | 'security' | 'network' | 'storage' | 'advanced' | 'about';

  let activeSection = $state<Section>('appearance');

  // Live preview: resolve current terminal theme to get colors
  let previewTheme = $derived(getTheme(resolveThemeKey(themeStore.terminalTheme, themeStore.mode)));

  const nav: { key: Section; label: string; icon: any }[] = [
    { key: 'appearance', label: 'Appearance', icon: Palette },
    { key: 'general', label: 'General', icon: GearSix },
    { key: 'security', label: 'Security', icon: Shield },
    { key: 'network', label: 'Network', icon: WifiHigh },
    { key: 'storage', label: 'Storage', icon: HardDrives },
    { key: 'advanced', label: 'Advanced', icon: Terminal },
    { key: 'about', label: 'About Capsem', icon: Info },
  ];
</script>

<div class="flex h-full">
  <!-- Left nav -->
  <aside class="w-56 shrink-0 border-e border-line-2 bg-background overflow-y-auto py-4">
    <h1 class="text-xl font-bold text-foreground px-5 mb-4">Settings</h1>
    <nav class="space-y-0.5 px-3">
      {#each nav as item (item.key)}
        <button
          type="button"
          class="w-full flex items-center gap-x-3 py-2 px-3 text-sm rounded-lg
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

  <!-- Content -->
  <main class="flex-1 overflow-y-auto py-6 px-8">
    {#if activeSection === 'appearance'}
      <h2 class="text-lg font-semibold text-foreground mb-6">Appearance</h2>

      <!-- ===== Interface ===== -->
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

      <!-- ===== Terminal ===== -->
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
              <span style="color: {previewTheme.green};">drwxr-xr-x</span><span style="color: {previewTheme.foreground};">  5 user user 160 Apr  9 </span><span style="color: {previewTheme.blue};">src/</span>
              <br>
              <span style="color: {previewTheme.foreground};">-rw-r--r--  1 user user  42 Apr  9 </span><span style="color: {previewTheme.foreground};">Cargo.toml</span>
              <br>
              <span style="color: {previewTheme.yellow};">-rwxr-xr-x</span><span style="color: {previewTheme.foreground};">  1 user user 8.2K Apr  9 </span><span style="color: {previewTheme.green};">build.sh</span>
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

    {:else if activeSection === 'general'}
      <h2 class="text-lg font-semibold text-foreground mb-6">General</h2>
      <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
        <div class="flex items-center justify-between p-4">
          <div>
            <p class="text-sm font-medium text-foreground">Default RAM</p>
            <p class="text-xs text-muted-foreground-1 mt-0.5">Memory allocated to new VMs</p>
          </div>
          <select class="py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary">
            <option>1024 MB</option>
            <option selected>2048 MB</option>
            <option>4096 MB</option>
            <option>8192 MB</option>
          </select>
        </div>
        <div class="flex items-center justify-between p-4">
          <div>
            <p class="text-sm font-medium text-foreground">Default CPUs</p>
            <p class="text-xs text-muted-foreground-1 mt-0.5">CPU cores allocated to new VMs</p>
          </div>
          <select class="py-2 px-3 text-sm rounded-lg border border-line-2 bg-layer text-foreground focus:outline-hidden focus:border-primary">
            <option>1</option>
            <option selected>2</option>
            <option>4</option>
            <option>8</option>
          </select>
        </div>
        <div class="flex items-center justify-between p-4">
          <div>
            <p class="text-sm font-medium text-foreground">Default persistence</p>
            <p class="text-xs text-muted-foreground-1 mt-0.5">Whether new VMs persist across restarts</p>
          </div>
          <button
            type="button"
            class="relative inline-flex shrink-0 h-6 w-11 border-2 border-transparent rounded-full bg-surface-2 cursor-pointer transition-colors"
            role="switch"
            aria-checked="false"
          >
            <span class="pointer-events-none inline-block size-5 rounded-full bg-layer shadow transform ring-0 transition translate-x-0"></span>
          </button>
        </div>
      </div>

    {:else if activeSection === 'security'}
      <h2 class="text-lg font-semibold text-foreground mb-6">Security</h2>
      <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
        <div class="flex items-center justify-between p-4">
          <div>
            <p class="text-sm font-medium text-foreground">Network isolation</p>
            <p class="text-xs text-muted-foreground-1 mt-0.5">Air-gap VMs from the host network by default</p>
          </div>
          <span class="text-xs px-2 py-0.5 rounded-full bg-primary text-primary-foreground">Enabled</span>
        </div>
        <div class="flex items-center justify-between p-4">
          <div>
            <p class="text-sm font-medium text-foreground">Read-only rootfs</p>
            <p class="text-xs text-muted-foreground-1 mt-0.5">Guest cannot modify its own system binaries</p>
          </div>
          <span class="text-xs px-2 py-0.5 rounded-full bg-primary text-primary-foreground">Enforced</span>
        </div>
      </div>

    {:else if activeSection === 'network'}
      <h2 class="text-lg font-semibold text-foreground mb-6">Network</h2>
      <div class="bg-card border border-card-line rounded-xl p-4">
        <p class="text-sm text-muted-foreground-1">MITM proxy settings, domain policies, and allowed endpoints.</p>
      </div>

    {:else if activeSection === 'storage'}
      <h2 class="text-lg font-semibold text-foreground mb-6">Storage</h2>
      <div class="bg-card border border-card-line rounded-xl p-4">
        <p class="text-sm text-muted-foreground-1">VM images, workspace directories, and disk usage.</p>
      </div>

    {:else if activeSection === 'advanced'}
      <h2 class="text-lg font-semibold text-foreground mb-6">Advanced</h2>
      <div class="bg-card border border-card-line rounded-xl p-4">
        <p class="text-sm text-muted-foreground-1">Service daemon, logging, and developer options.</p>
      </div>

    {:else if activeSection === 'about'}
      <h2 class="text-lg font-semibold text-foreground mb-6">About Capsem</h2>
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
    {/if}
  </main>
</div>
