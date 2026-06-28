<script lang="ts">
  import { onMount } from 'svelte';
  import { themeStore, PRELINE_THEMES, FONT_SIZES, FONT_FAMILIES, UI_FONT_SIZES } from '../../stores/theme.svelte.ts';
  import { settingsStore } from '../../stores/settings.svelte.ts';
  import { THEME_FAMILIES, getTheme, resolveThemeKey } from '../../terminal/themes';
  import * as api from '../../api';
  import type { UpdateStatusResponse, UpdateTrackStatus } from '../../types/gateway';
  import {
    UPDATE_TRACK_LABELS,
    updateEvidenceLinks,
    updateSummary,
    updateTrackDetail,
    updateTrackVersion,
    type UpdateTrackKey,
  } from '../../models/update-status';
  import SettingsSection from '../settings/SettingsSection.svelte';
  import Palette from 'phosphor-svelte/lib/Palette';
  import GearSix from 'phosphor-svelte/lib/GearSix';
  import Desktop from 'phosphor-svelte/lib/Desktop';
  import Info from 'phosphor-svelte/lib/Info';
  import Sun from 'phosphor-svelte/lib/Sun';
  import Moon from 'phosphor-svelte/lib/Moon';
  import Export from 'phosphor-svelte/lib/Export';
  import DownloadSimple from 'phosphor-svelte/lib/DownloadSimple';

  // Live preview: resolve current terminal theme to get colors
  let previewTheme = $derived(getTheme(resolveThemeKey(themeStore.terminalTheme, themeStore.mode)));

  // Active section (panel-per-section, not scrollspy)
  let activeSection = $state('appearance');

  // Dynamic sections from settings tree (UI/app preferences only).
  let dynamicSections = $derived.by(() => {
    const sections = settingsStore.model?.sections ?? [];
    return sections.filter(s =>
      s.key !== 'appearance'
      && s.key !== 'app'
      && !['ai', 'repository', 'security', 'vm', 'mcp', 'plugins'].includes(s.key)
    );
  });

  // Active dynamic group (if sidebar selected a dynamic section)
  let activeDynamicGroup = $derived.by(() => {
    return dynamicSections.find(s => s.key === activeSection);
  });

  // Icon map for dynamic sections
  const SECTION_ICONS: Record<string, any> = {
    app: GearSix,
  };

  // Build full nav list: Appearance + settings-owned dynamic sections + About.
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
    items.push({ key: 'about', label: 'About', icon: Info });
    return items;
  });

  let diagnostics = $state<Record<string, any> | null>(null);
  let diagnosticsError = $state<string | null>(null);
  let diagnosticsCopied = $state(false);
  let updateStatus = $derived.by(() => {
    const value = diagnostics?.update_status;
    return isUpdateStatus(value) ? value : null;
  });
  let updateStatusError = $derived.by(() => {
    const value = diagnostics?.update_status;
    if (!value || isUpdateStatus(value)) return null;
    if (typeof value === 'object' && 'error' in value) return String((value as { error: unknown }).error);
    return 'Update status unavailable';
  });

  onMount(() => {
    settingsStore.load();
    refreshDiagnostics();
  });

  let importInput = $state<HTMLInputElement>(null!);
  let importMessage = $state<{ text: string; error: boolean } | null>(null);

  async function handleSave() {
    await settingsStore.save();
  }

  async function handleDiscard() {
    await settingsStore.discard();
  }

  function handleExport() {
    settingsStore.exportSettings();
  }

  async function handleImport(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    importMessage = null;
    try {
      const count = await settingsStore.importSettings(file);
      importMessage = count > 0
        ? { text: `${count} setting${count === 1 ? '' : 's'} staged. Review and save to apply.`, error: false }
        : { text: 'No changes -- imported settings match current values.', error: false };
    } catch (err) {
      importMessage = { text: String(err instanceof Error ? err.message : err), error: true };
    }
    input.value = '';
  }

  async function refreshDiagnostics() {
    diagnosticsError = null;
    try {
      diagnostics = await api.debugSnapshot() as Record<string, any>;
    } catch (err) {
      diagnosticsError = err instanceof Error ? err.message : String(err);
    }
  }

  async function copyDiagnostics() {
    const snapshot = diagnostics ?? (await api.debugSnapshot() as Record<string, any>);
    await navigator.clipboard.writeText(JSON.stringify(snapshot, null, 2));
    diagnosticsCopied = true;
    window.setTimeout(() => { diagnosticsCopied = false; }, 1500);
  }

  function isUpdateStatus(value: unknown): value is UpdateStatusResponse {
    return !!value
      && typeof value === 'object'
      && 'binary' in value
      && 'assets' in value
      && 'profiles' in value
      && 'images' in value;
  }

  function checkedAtLabel(value: number | null | undefined): string {
    if (!value) return 'never';
    return new Date(value * 1000).toLocaleString();
  }

  function trackStateLabel(track: UpdateTrackStatus): string {
    if (track.update_available) return 'Update available';
    if (track.state === 'not_published') return 'Not published';
    if (track.state === 'unknown') return 'Unknown';
    return 'Current';
  }

  function trackRow(status: UpdateStatusResponse, key: UpdateTrackKey): UpdateTrackStatus {
    return status[key];
  }

  function evidenceHref(href: string): string {
    if (href.startsWith('http://') || href.startsWith('https://')) return href;
    if (href.startsWith('/')) return `${api.getBaseUrl()}${href}`;
    const channel = updateStatus?.channel_url;
    if (channel?.startsWith('http://') || channel?.startsWith('https://')) {
      return new URL(href, channel).toString();
    }
    return href;
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
    {#if settingsStore.loading && !settingsStore.model}
      <div class="flex items-center justify-center h-full">
        <div class="animate-spin size-6 border-2 border-primary border-t-transparent rounded-full"></div>
      </div>
    {:else if settingsStore.error && !settingsStore.model}
      <div class="flex flex-col items-center justify-center h-full gap-y-4">
        <p class="text-sm text-destructive-foreground">{settingsStore.error}</p>
        <button
          type="button"
          class="py-2 px-4 text-sm font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover transition-colors"
          onclick={() => settingsStore.load()}
        >
          Retry
        </button>
      </div>
    {:else}
    <div class="py-6 px-8">

      {#if activeSection === 'appearance'}
        <!-- ===== Appearance (custom, not from backend tree) ===== -->
        <h2 class="text-xl font-medium text-foreground mb-6">Appearance</h2>

        <!-- Interface -->
        <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2">Interface</h3>
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
        <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2">Terminal</h3>
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

      {:else if activeSection === 'about'}
        <!-- ===== About ===== -->
        <h2 class="text-xl font-medium text-foreground mb-6">About</h2>

        <!-- App settings (auto-update, check for updates) -->
        {@const appGroup = settingsStore.findGroup('App')}
        {#if appGroup}
          <SettingsSection group={appGroup} depth={1} />
        {/if}

        <!-- Release channel -->
        <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2 mt-6">Release channel</h3>
        <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
          <div class="flex items-center justify-between p-4 gap-x-4">
            <div>
              <p class="text-sm font-medium text-foreground">Update status</p>
              <p class="text-xs text-muted-foreground-1 mt-0.5">
                {#if updateStatus}
                  {updateStatus.channel_url ?? 'default channel'} · checked {checkedAtLabel(updateStatus.checked_at)}
                {:else if updateStatusError}
                  {updateStatusError}
                {:else}
                  Checking release channel
                {/if}
              </p>
              {#if updateStatus?.last_error}
                <p class="text-xs text-destructive mt-1">{updateStatus.last_error}</p>
              {/if}
            </div>
            <div class="flex items-center gap-x-2">
              {#if updateStatus}
                <span class="text-xs px-2 py-1 rounded-lg bg-primary/10 text-primary">
                  {updateSummary(updateStatus)}
                </span>
              {:else}
                <span class="text-xs px-2 py-1 rounded-lg bg-destructive/10 text-destructive">
                  Unavailable
                </span>
              {/if}
              <button
                type="button"
                class="py-2 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg border border-line-2 bg-layer text-foreground hover:bg-layer-hover transition-colors"
                onclick={refreshDiagnostics}
              >
                Refresh
              </button>
            </div>
          </div>
          {#if updateStatus}
            {#each (['binary', 'assets', 'profiles', 'images'] as UpdateTrackKey[]) as key (key)}
              {@const track = trackRow(updateStatus, key)}
              {@const detail = updateTrackDetail(track)}
              <div class="flex items-center justify-between p-4 gap-x-4">
                <div>
                  <p class="text-sm text-foreground">{UPDATE_TRACK_LABELS[key]}</p>
                  <p class="text-xs text-muted-foreground-1 mt-0.5">{updateTrackVersion(track)}</p>
                  {#if detail}
                    <p class="text-xs text-muted-foreground-1 mt-1">{detail}</p>
                  {/if}
                </div>
                <p class="text-sm {track.update_available ? 'text-primary' : 'text-muted-foreground-1'}">
                  {trackStateLabel(track)}
                </p>
              </div>
            {/each}
            {@const evidenceLinks = updateEvidenceLinks(updateStatus)}
            {#if evidenceLinks.length > 0}
              <div class="p-4">
                <p class="text-sm font-medium text-foreground mb-3">Release evidence</p>
                <div class="grid grid-cols-1 md:grid-cols-2 gap-2">
                  {#each evidenceLinks as link (`${link.label}:${link.href}`)}
                    <a
                      class="flex items-center justify-between gap-x-3 rounded-lg border border-line-2 bg-layer px-3 py-2 text-sm text-foreground hover:bg-layer-hover transition-colors"
                      href={evidenceHref(link.href)}
                      target="_blank"
                      rel="noopener noreferrer"
                    >
                      <span>{link.label}</span>
                      {#if link.meta}
                        <span class="text-xs text-muted-foreground-1">{link.meta}</span>
                      {/if}
                    </a>
                  {/each}
                </div>
              </div>
            {/if}
          {/if}
        </div>

        <!-- Diagnostics -->
        <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2 mt-6">Diagnostics</h3>
        <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
          <div class="flex items-center justify-between p-4">
            <p class="text-sm text-foreground">Service</p>
            <p class="text-sm text-muted-foreground-1">{diagnostics?.status?.service ?? 'unknown'}</p>
          </div>
          <div class="flex items-center justify-between p-4">
            <p class="text-sm text-foreground">Gateway version</p>
            <p class="text-sm text-muted-foreground-1">{diagnostics?.status?.gateway_version ?? 'unknown'}</p>
          </div>
          <div class="flex items-center justify-between p-4">
            <p class="text-sm text-foreground">Profiles</p>
            <p class="text-sm text-muted-foreground-1">
              {diagnostics?.profiles_status?.ready_count ?? 0}/{diagnostics?.profiles_status?.profile_count ?? 0} ready
            </p>
          </div>
          <div class="flex items-center justify-between p-4">
            <p class="text-sm text-foreground">Corp</p>
            <p class="text-sm text-muted-foreground-1">
              {diagnostics?.corp_info?.installed ? 'installed' : 'not installed'}
            </p>
          </div>
          <div class="flex items-center justify-between p-4">
            <div>
              <p class="text-sm font-medium text-foreground">Debug snapshot</p>
              <p class="text-xs text-muted-foreground-1 mt-0.5">
                Service, profile, corp, and VM status for bug reports
              </p>
              {#if diagnosticsError}
                <p class="text-xs text-destructive mt-1">{diagnosticsError}</p>
              {/if}
            </div>
            <div class="flex items-center gap-x-2">
              <button
                type="button"
                class="py-2 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg border border-line-2 bg-layer text-foreground hover:bg-layer-hover transition-colors"
                onclick={refreshDiagnostics}
              >
                Refresh
              </button>
              <button
                type="button"
                class="py-2 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg border border-line-2 bg-layer text-foreground hover:bg-layer-hover transition-colors"
                onclick={copyDiagnostics}
              >
                {diagnosticsCopied ? 'Copied' : 'Copy'}
              </button>
            </div>
          </div>
        </div>

        <!-- Data management -->
        <h3 class="text-xs font-semibold text-foreground uppercase tracking-wider mb-2 mt-6">Data</h3>
        <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider">
          <div class="flex items-center justify-between p-4">
            <div>
              <p class="text-sm font-medium text-foreground">Export settings</p>
              <p class="text-xs text-muted-foreground-1 mt-0.5">Download all settings as a JSON file</p>
            </div>
            <button
              type="button"
              class="py-2 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg border border-line-2 bg-layer text-foreground hover:bg-layer-hover transition-colors"
              onclick={handleExport}
            >
              <Export size={16} />
              Export
            </button>
          </div>
          <div class="p-4">
            <div class="flex items-center justify-between">
              <div>
                <p class="text-sm font-medium text-foreground">Import settings</p>
                <p class="text-xs text-muted-foreground-1 mt-0.5">Load settings from a previously exported JSON file</p>
              </div>
              <button
                type="button"
                class="py-2 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg border border-line-2 bg-layer text-foreground hover:bg-layer-hover transition-colors"
                onclick={() => importInput.click()}
              >
                <DownloadSimple size={16} />
                Import
              </button>
              <input
                bind:this={importInput}
                type="file"
                accept=".json"
                class="hidden"
                onchange={handleImport}
              />
            </div>
            {#if importMessage}
              <p class="text-xs mt-2 {importMessage.error ? 'text-destructive-foreground' : 'text-muted-foreground-1'}">
                {importMessage.text}
              </p>
            {/if}
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
    {/if}
  </main>
</div>
