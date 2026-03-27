<script lang="ts">
  import type { ConfigIssue, SettingsGroup, SettingsLeaf, SettingsAction, SettingsNode, SettingValue, UpdateInfo } from '../../types';
  import { settingsStore } from '../../stores/settings.svelte';
  import { themeStore } from '../../stores/theme.svelte';
  import { openUrl, checkForAppUpdate } from '../../api';
  import Self from './SettingsSection.svelte';
  import PresetSection from './PresetSection.svelte';
  import { wizardStore } from '../../stores/wizard.svelte';
  import { SideEffect, ActionKind } from '../../models/settings-enums';

  let { group, depth = 0 }: { group: SettingsGroup; depth?: number } = $props();

  // Update check state (only used for App group).
  let updateChecking = $state(false);
  let updateResult = $state<UpdateInfo | null | 'none' | 'error'>(null);

  async function handleCheckUpdate() {
    updateChecking = true;
    updateResult = null;
    try {
      const info = await checkForAppUpdate();
      updateResult = info ?? 'none';
    } catch {
      updateResult = 'error';
    } finally {
      updateChecking = false;
    }
  }

  /** Extract path + content from a File setting value. */
  function fileValue(v: SettingValue): { path: string; content: string } {
    if (typeof v === 'object' && v !== null && 'path' in v) return v as { path: string; content: string };
    console.error('fileValue: expected { path, content } but got', typeof v, v);
    return { path: '', content: String(v) };
  }

  // Track collapsed state for sub-groups with enabled_by toggle.
  let expandedGroups = $state<Set<string>>(new Set());
  // Track API key reveal state.
  let revealedKeys = $state<Set<string>>(new Set());
  // Track which file settings are in edit mode.
  let editingFiles = $state<Set<string>>(new Set());
  // Track draft text for file settings while editing.
  let fileDrafts = $state<Map<string, string>>(new Map());
  // Track draft guest paths for file settings while editing.
  let pathDrafts = $state<Map<string, string>>(new Map());
  // Track "copied" feedback state.
  let copiedId = $state<string | null>(null);
  // Track domain list chip input drafts.
  let chipInputs = $state<Map<string, string>>(new Map());

  /** Parse comma-separated domain string into trimmed array. */
  function parseDomains(text: string): string[] {
    return text.split(',').map(d => d.trim()).filter(d => d.length > 0);
  }

  /** Join domain array back to comma-separated string. */
  function joinDomains(domains: string[]): string {
    return domains.join(', ');
  }

  function removeDomain(id: string, current: string, domain: string) {
    const domains = parseDomains(current).filter(d => d !== domain);
    handleUpdate(id, joinDomains(domains));
  }

  function addDomain(id: string, current: string) {
    const input = (chipInputs.get(id) ?? '').trim();
    if (!input) return;
    const domains = parseDomains(current);
    // Split on comma in case user pasted multiple
    const newDomains = input.split(',').map(d => d.trim()).filter(d => d.length > 0);
    for (const d of newDomains) {
      if (!domains.includes(d)) domains.push(d);
    }
    const next = new Map(chipInputs);
    next.set(id, '');
    chipInputs = next;
    handleUpdate(id, joinDomains(domains));
  }

  function handleChipKeydown(e: KeyboardEvent, id: string, current: string) {
    if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault();
      addDomain(id, current);
    }
  }

  function toggleGroup(key: string) {
    const next = new Set(expandedGroups);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    expandedGroups = next;
  }

  function toggleReveal(id: string) {
    const next = new Set(revealedKeys);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    revealedKeys = next;
  }

  function startEditing(id: string, fv: { path: string; content: string }, filetype: string) {
    const next = new Set(editingFiles);
    next.add(id);
    editingFiles = next;
    const drafts = new Map(fileDrafts);
    drafts.set(id, formatContent(fv.content, filetype));
    fileDrafts = drafts;
    const paths = new Map(pathDrafts);
    paths.set(id, fv.path);
    pathDrafts = paths;
  }

  function cancelEditing(id: string) {
    const next = new Set(editingFiles);
    next.delete(id);
    editingFiles = next;
    const drafts = new Map(fileDrafts);
    drafts.delete(id);
    fileDrafts = drafts;
    const paths = new Map(pathDrafts);
    paths.delete(id);
    pathDrafts = paths;
  }

  async function copyContent(text: string, id: string) {
    try {
      await navigator.clipboard.writeText(text);
      copiedId = id;
      setTimeout(() => { if (copiedId === id) copiedId = null; }, 1500);
    } catch { /* clipboard may not be available */ }
  }

  async function saveFile(id: string, filetype: string) {
    const draft = fileDrafts.get(id) ?? '';
    const path = pathDrafts.get(id) ?? '';
    const next = new Set(editingFiles);
    next.delete(id);
    editingFiles = next;
    const drafts = new Map(fileDrafts);
    drafts.delete(id);
    fileDrafts = drafts;
    const paths = new Map(pathDrafts);
    paths.delete(id);
    pathDrafts = paths;
    const compacted = compactContent(draft, filetype);
    await handleUpdate(id, { path, content: compacted });
  }

  // Find the toggle leaf within a group's children (the one matching enabled_by).
  function findToggle(children: SettingsNode[], enabledBy: string | null | undefined): SettingsLeaf | null {
    if (!enabledBy) return null;
    for (const child of children) {
      if (child.kind === 'leaf' && child.id === enabledBy) return child;
    }
    return null;
  }

  // Check if a provider/parent toggle is on.
  function isToggleOn(children: SettingsNode[], enabledBy: string | null | undefined): boolean {
    const toggle = findToggle(children, enabledBy);
    if (!toggle) return true;
    return toggle.effective_value === true;
  }

  /** Collect all lint issues for all leaves inside a group (recursive). */
  function groupIssues(children: SettingsNode[]): ConfigIssue[] {
    const issues: ConfigIssue[] = [];
    for (const child of children) {
      if (child.kind === 'leaf') {
        issues.push(...settingsStore.issuesFor(child.id));
      } else if (child.kind === 'group') {
        issues.push(...groupIssues(child.children));
      }
    }
    return issues;
  }

  async function handleUpdate(id: string, value: unknown) {
    // Side effect dispatch (grammar-driven, no hardcoded IDs)
    const leaf = settingsStore.findLeaf(id);
    if (leaf?.metadata.side_effect === SideEffect.ToggleTheme) {
      themeStore.toggle();
    }
    // Toggles save immediately; other types accumulate for batch save
    if (leaf?.setting_type === 'bool') {
      await settingsStore.updateImmediate(id, value as SettingValue);
    } else {
      settingsStore.stage(id, value as SettingValue);
    }
  }

  /** Detect filetype from metadata or path extension. */
  function detectFiletype(s: SettingsLeaf): string {
    if (s.metadata.filetype) return s.metadata.filetype;
    const fv = fileValue(s.effective_value);
    const ext = fv.path.split('.').pop()?.toLowerCase() ?? '';
    if (ext === 'json') return 'json';
    if (ext === 'sh' || ext === 'bashrc' || fv.path.endsWith('.bashrc')) return 'bash';
    if (ext === 'conf') return 'conf';
    return 'text';
  }

  /** Format content for display (pretty-print JSON, pass-through others). */
  function formatContent(text: string, filetype: string): string {
    if (filetype === 'json') {
      try {
        return JSON.stringify(JSON.parse(text), null, 2);
      } catch {
        return text;
      }
    }
    return text;
  }

  /** Compact content for storage (minify JSON, pass-through others). */
  function compactContent(text: string, filetype: string): string {
    if (filetype === 'json') {
      try {
        return JSON.stringify(JSON.parse(text));
      } catch {
        return text.trim();
      }
    }
    return text.trim();
  }

  /** Escape HTML entities. */
  function escapeHtml(text: string): string {
    return text
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
  }

  /** JSON syntax highlighter. */
  function highlightJson(text: string): string {
    const formatted = formatContent(text, 'json');
    const escaped = escapeHtml(formatted);
    return escaped
      .replace(
        /("(?:[^"\\]|\\.)*")(\s*:)?/g,
        (match, str, colon) => {
          if (colon) {
            return `<span class="json-key">${str}</span>${colon}`;
          }
          return `<span class="json-string">${str}</span>`;
        },
      )
      .replace(/\b(true|false|null)\b/g, '<span class="json-bool">$1</span>')
      .replace(/\b(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)\b/g, '<span class="json-number">$1</span>');
  }

  /** Bash/shell syntax highlighter (single-pass to avoid HTML tag interference). */
  function highlightBash(text: string): string {
    const escaped = escapeHtml(text);
    return escaped.split('\n').map(line => {
      if (/^\s*#/.test(line)) {
        return `<span class="sh-comment">${line}</span>`;
      }
      // Single-pass: all token types in one regex so matches cannot overlap
      return line.replace(
        /('(?:[^'\\]|\\.)*')|("(?:[^"\\]|\\.)*")|(\\033\[[0-9;]*m|\\e\[[0-9;]*m)|(\$\{[^}]+\}|\$[A-Za-z_][A-Za-z0-9_]*)|\b(alias|export|if|then|else|elif|fi|for|do|done|case|esac|function|while|until|in|local|return|source)\b|(#.*$)/g,
        (match, single, double, escape, variable, keyword, comment) => {
          if (single || double) return `<span class="json-string">${match}</span>`;
          if (escape) return `<span class="sh-escape">${match}</span>`;
          if (variable) return `<span class="sh-variable">${match}</span>`;
          if (keyword) return `<span class="sh-keyword">${match}</span>`;
          if (comment) return `<span class="sh-comment">${match}</span>`;
          return match;
        },
      );
    }).join('\n');
  }

  /** Config file (tmux/generic) syntax highlighter (single-pass). */
  function highlightConf(text: string): string {
    const escaped = escapeHtml(text);
    return escaped.split('\n').map(line => {
      if (/^\s*#/.test(line)) {
        return `<span class="sh-comment">${line}</span>`;
      }
      // Single-pass: strings, flags, keywords, comments
      return line.replace(
        /("(?:[^"\\]|\\.)*")|('(?:[^'\\]|\\.)*')|\b(set|bind|unbind|source|run|if|set-option|set-window-option|bind-key|unbind-key)\b|(#.*$)/g,
        (match, dbl, sgl, keyword, comment) => {
          if (dbl || sgl) return `<span class="json-string">${match}</span>`;
          if (keyword) return `<span class="sh-keyword">${match}</span>`;
          if (comment) return `<span class="sh-comment">${match}</span>`;
          return match;
        },
      );
    }).join('\n');
  }

  /** Dispatch syntax highlighting by filetype. */
  function highlightContent(text: string, filetype: string): string {
    switch (filetype) {
      case 'json': return highlightJson(text);
      case 'bash': return highlightBash(text);
      case 'conf': return highlightConf(text);
      default: return escapeHtml(text);
    }
  }
</script>

{#snippet fileControl(s: SettingsLeaf)}
  {@const issues = settingsStore.issuesFor(s.id)}
  {@const disabled = s.corp_locked || !s.enabled}
  {@const isEditing = editingFiles.has(s.id)}
  {@const fv = fileValue(s.effective_value)}
  {@const filetype = detectFiletype(s)}
  <div class="card card-bordered card-compact bg-base-200/30 overflow-hidden">
    <!-- File header bar -->
    <div class="flex items-center gap-2 px-3 py-1.5 border-b border-base-300/50 bg-base-200/50">
      <svg class="size-3.5 text-base-content/40 flex-shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>
      <span class="text-xs font-medium flex-1 min-w-0 truncate">{s.name}</span>
      {#if fv.path || isEditing}
        {#if isEditing}
          <input
            type="text"
            class="text-[10px] font-mono text-base-content/50 bg-transparent border-b border-base-content/20 focus:outline-none focus:border-interactive w-48 text-right px-0.5"
            value={pathDrafts.get(s.id) ?? fv.path}
            oninput={(e) => {
              const paths = new Map(pathDrafts);
              paths.set(s.id, e.currentTarget.value);
              pathDrafts = paths;
            }}
          />
        {:else}
          <span class="text-[10px] font-mono text-base-content/30 truncate max-w-48">{fv.path}</span>
        {/if}
      {/if}
      {#if s.corp_locked}
        <span class="badge badge-xs bg-denied/15 text-denied">corp</span>
      {/if}
      <!-- Copy button -->
      <button
        class="btn btn-ghost btn-xs text-base-content/40"
        onclick={() => copyContent(isEditing ? (fileDrafts.get(s.id) ?? fv.content) : fv.content, s.id)}
        title="Copy to clipboard"
      >
        {#if copiedId === s.id}
          <svg class="size-3.5 text-allowed" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="20 6 9 17 4 12" /></svg>
        {:else}
          <svg class="size-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" /></svg>
        {/if}
      </button>
      {#if !disabled}
        {#if isEditing}
          <button
            class="btn btn-ghost btn-xs text-base-content/50"
            onclick={() => cancelEditing(s.id)}
          >Cancel</button>
          <button
            class="btn bg-interactive text-white btn-xs"
            onclick={() => saveFile(s.id, filetype)}
          >Save</button>
        {:else}
          <button
            class="btn btn-ghost btn-xs text-base-content/50"
            onclick={() => startEditing(s.id, fv, filetype)}
          >Edit</button>
        {/if}
      {/if}
    </div>
    <!-- File content -->
    {#if isEditing}
      <textarea
        class="w-full bg-transparent font-mono text-xs leading-relaxed p-3 focus:outline-none resize-y min-h-20"
        rows={Math.min(Math.max(formatContent(fv.content, filetype).split('\n').length + 1, 4), 20)}
        value={fileDrafts.get(s.id) ?? formatContent(fv.content, filetype)}
        oninput={(e) => {
          const drafts = new Map(fileDrafts);
          drafts.set(s.id, e.currentTarget.value);
          fileDrafts = drafts;
        }}
      ></textarea>
    {:else}
      <pre class="font-mono text-xs leading-relaxed p-3 overflow-x-auto whitespace-pre-wrap">{@html highlightContent(fv.content, filetype)}</pre>
    {/if}
    <!-- Lint issues -->
    {#if issues.length > 0}
      <div class="px-3 pb-2 flex flex-col gap-0.5">
        {#each issues as issue}
          <span class="text-xs {issue.severity === 'error' ? 'text-denied' : 'text-caution'}">
            {issue.message}
          </span>
        {/each}
      </div>
    {/if}
  </div>
{/snippet}

{#snippet domainListControl(s: SettingsLeaf)}
  {@const issues = settingsStore.issuesFor(s.id)}
  {@const disabled = s.corp_locked || !s.enabled}
  {@const domains = parseDomains(String(s.effective_value))}
  <div class="form-control">
    <div class="flex items-center gap-2 mb-0.5">
      <span class="text-sm font-medium">{s.name}</span>
      {#if s.corp_locked}
        <span class="badge badge-xs bg-denied/15 text-denied">corp</span>
      {/if}
    </div>
    {#if s.description}
      <p class="text-xs text-base-content/50 mb-1.5">{s.description}</p>
    {/if}
    <div class="flex flex-wrap gap-1.5 items-center">
      {#each domains as domain}
        <span class="badge badge-sm bg-base-200 gap-1 font-mono text-xs">
          {domain}
          {#if !disabled}
            <button
              class="text-base-content/40 hover:text-base-content/70"
              onclick={() => removeDomain(s.id, String(s.effective_value), domain)}
              title="Remove {domain}"
            >
              <svg class="size-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
            </button>
          {/if}
        </span>
      {/each}
      {#if !disabled}
        <input
          type="text"
          class="input input-xs input-bordered w-40 font-mono text-xs"
          placeholder="add domain..."
          value={chipInputs.get(s.id) ?? ''}
          oninput={(e) => {
            const next = new Map(chipInputs);
            next.set(s.id, e.currentTarget.value);
            chipInputs = next;
          }}
          onkeydown={(e) => handleChipKeydown(e, s.id, String(s.effective_value))}
        />
      {/if}
    </div>
    {#if issues.length > 0}
      <div class="mt-1 flex flex-col gap-0.5">
        {#each issues as issue}
          <span class="text-xs {issue.severity === 'error' ? 'text-denied' : 'text-caution'}">
            {issue.message}
          </span>
        {/each}
      </div>
    {/if}
  </div>
{/snippet}

{#snippet leafControl(s: SettingsLeaf)}
  {@const issues = settingsStore.issuesFor(s.id)}
  {@const disabled = s.corp_locked || !s.enabled}
  {#if s.setting_type === 'file'}
    {@render fileControl(s)}
  {:else if s.setting_type === 'text' && s.metadata.format === 'domain_list'}
    {@render domainListControl(s)}
  {:else}
  <div class="form-control">
    <div class="flex items-start gap-3">
      <div class="flex-1 min-w-0">
        <div class="flex items-center gap-2 mb-0.5">
          <span class="text-sm font-medium">{s.name}</span>
          {#if s.corp_locked}
            <span class="badge badge-xs bg-denied/15 text-denied">corp</span>
          {/if}
        </div>
        {#if s.description}
          <p class="text-xs text-base-content/50 mb-1.5">{s.description}</p>
        {/if}
      </div>
      <div class="flex-shrink-0">
        {#if s.setting_type === 'bool'}
          <input
            type="checkbox"
            class="toggle toggle-sm"
            checked={s.effective_value === true}
            {disabled}
            onchange={(e) => handleUpdate(s.id, e.currentTarget.checked)}
          />
        {:else if s.setting_type === 'apikey' || s.metadata?.mask}
          <div class="flex flex-col items-end gap-0.5">
            <div class="flex items-center gap-1">
              <input
                type={revealedKeys.has(s.id) ? 'text' : 'password'}
                class="input input-sm input-bordered w-64 font-mono text-xs"
                value={String(s.effective_value)}
                {disabled}
                placeholder={s.metadata.prefix ? `${s.metadata.prefix}...` : ''}
                onchange={(e) => handleUpdate(s.id, e.currentTarget.value)}
              />
              <button
                class="btn btn-ghost btn-xs"
                onclick={() => toggleReveal(s.id)}
                title={revealedKeys.has(s.id) ? 'Hide' : 'Show'}
              >
                {#if revealedKeys.has(s.id)}
                  <svg class="size-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24" /><line x1="1" y1="1" x2="23" y2="23" /></svg>
                {:else}
                  <svg class="size-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" /><circle cx="12" cy="12" r="3" /></svg>
                {/if}
              </button>
            </div>
            {#if s.metadata.prefix && String(s.effective_value) && !String(s.effective_value).startsWith(s.metadata.prefix)}
              <span class="text-xs text-caution">Token should start with {s.metadata.prefix}</span>
            {/if}
          </div>
        {:else if s.setting_type === 'number'}
          <div class="flex flex-col items-end gap-0.5">
            <input
              type="number"
              class="input input-sm input-bordered w-28 text-right font-mono text-xs"
              value={Number(s.effective_value)}
              min={s.metadata.min ?? undefined}
              max={s.metadata.max ?? undefined}
              {disabled}
              onchange={(e) => handleUpdate(s.id, Number(e.currentTarget.value))}
            />
            {#if s.metadata.min !== null || s.metadata.max !== null}
              <span class="text-[10px] text-base-content/40">
                {s.metadata.min ?? ''} -- {s.metadata.max ?? ''}
              </span>
            {/if}
          </div>
        {:else if s.setting_type === 'text' && s.metadata.choices.length > 0}
          <select
            class="select select-sm select-bordered w-40 text-xs"
            value={String(s.effective_value)}
            {disabled}
            onchange={(e) => handleUpdate(s.id, e.currentTarget.value)}
          >
            {#each s.metadata.choices as choice}
              <option value={choice}>{choice}</option>
            {/each}
          </select>
        {:else}
          <input
            type="text"
            class="input input-sm input-bordered w-64 font-mono text-xs"
            value={String(s.effective_value)}
            {disabled}
            onchange={(e) => handleUpdate(s.id, e.currentTarget.value)}
          />
        {/if}
      </div>
    </div>
    <!-- Lint issues -->
    {#if issues.length > 0}
      <div class="mt-1 flex flex-col gap-0.5">
        {#each issues as issue}
          <span class="text-xs {issue.severity === 'error' ? 'text-denied' : 'text-caution'}">
            {issue.message}
          </span>
        {/each}
      </div>
    {/if}
  </div>
  {/if}
{/snippet}

{#snippet actionControl(a: SettingsAction)}
  {#if a.action === ActionKind.CheckUpdate}
    <div class="form-control">
      <div class="flex items-start gap-3">
        <div class="flex-1 min-w-0">
          <div class="flex items-center gap-2 mb-0.5">
            <span class="text-sm font-medium">{a.name}</span>
          </div>
          {#if a.description}
            <p class="text-xs text-base-content/50 mb-1.5">{a.description}</p>
          {/if}
          {#if updateResult === 'none'}
            <p class="text-xs text-allowed">You are on the latest version.</p>
          {:else if updateResult === 'error'}
            <p class="text-xs text-denied">Update check failed.</p>
          {:else if updateResult && typeof updateResult === 'object'}
            <p class="text-xs text-interactive">Version {updateResult.version} is available (current: {updateResult.current_version}).</p>
          {/if}
        </div>
        <div class="flex-shrink-0">
          <button
            class="btn btn-sm btn-outline"
            disabled={updateChecking}
            onclick={handleCheckUpdate}
          >
            {#if updateChecking}
              <span class="loading loading-spinner loading-xs"></span>
              Checking...
            {:else}
              Check now
            {/if}
          </button>
        </div>
      </div>
    </div>
  {:else if a.action === ActionKind.PresetSelect}
    <div id="settings-group-Preset" data-subgroup="Preset" class="mt-6 first:mt-0 mb-2 scroll-mt-4">
      <h2 class="text-lg font-semibold text-interactive mb-0.5">{a.name}</h2>
      {#if a.description}
        <p class="text-xs text-base-content/50 mb-2">{a.description}</p>
      {/if}
      <PresetSection />
    </div>
  {:else if a.action === ActionKind.RerunWizard}
    <div class="mt-8 pt-4 border-t border-base-200">
      <div class="flex items-center justify-between">
        <div>
          <span class="text-sm font-medium">{a.name}</span>
          {#if a.description}
            <p class="text-xs text-base-content/50">{a.description}</p>
          {/if}
        </div>
        <button
          class="btn btn-ghost btn-sm"
          onclick={() => wizardStore.rerun()}
        >
          Re-run Wizard
        </button>
      </div>
    </div>
  {/if}
{/snippet}

<!-- Top-level group header -->
{#if depth === 0}
  <div class="mb-4">
    <h1 class="text-2xl font-bold">{group.name}</h1>
    {#if group.description}
      <p class="text-sm text-base-content/50">{group.description}</p>
    {/if}
  </div>
{/if}

<!-- Render children -->
{#each group.children as child}
  {#if child.kind === 'action'}
    <div class="py-2 border-b border-base-200 last:border-b-0">
      {@render actionControl(child)}
    </div>
  {:else if child.kind === 'leaf'}
    <div class="py-2 border-b border-base-200 last:border-b-0">
      {@render leafControl(child)}
    </div>
  {:else if child.kind === 'group'}
    {@const hasToggle = !!child.enabled_by}
    {@const toggle = findToggle(child.children, child.enabled_by)}
    {@const isOn = isToggleOn(child.children, child.enabled_by)}
    {@const contentChildren = child.children.filter(c => !(c.kind === 'leaf' && c.id === child.enabled_by))}
    {@const isExpanded = expandedGroups.has(child.key) || !hasToggle}

    {#if hasToggle}
      {@const headerIssues = groupIssues(child.children)}
      <!-- Toggle-gated group: card with toggle header -->
      <div id="settings-group-{child.name}" data-subgroup={child.name} class="card card-bordered mb-3 overflow-hidden">
        <div class="flex items-center gap-3 px-4 py-2.5 bg-base-200/40">
          {#if toggle}
            <input
              type="checkbox"
              class="toggle toggle-sm"
              checked={toggle.effective_value === true}
              disabled={toggle.corp_locked}
              onchange={(e) => handleUpdate(toggle.id, e.currentTarget.checked)}
            />
          {/if}
          <button
            class="flex-1 text-left"
            onclick={() => toggleGroup(child.key)}
          >
            <span class="text-sm font-medium">{child.name}</span>
            {#if toggle?.corp_locked}
              <span class="badge badge-xs bg-denied/15 text-denied ml-1">corp</span>
            {/if}
            {#if child.description}
              <span class="text-xs text-base-content/40 ml-2">{child.description}</span>
            {/if}
          </button>
          <button
            class="btn btn-ghost btn-xs"
            aria-label={isExpanded ? 'Collapse' : 'Expand'}
            onclick={() => toggleGroup(child.key)}
          >
            <svg class="size-3 transition-transform {isExpanded ? 'rotate-180' : ''}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="6 9 12 15 18 9" /></svg>
          </button>
        </div>
        {#if headerIssues.length > 0 && !isExpanded}
          <div class="flex flex-col gap-0.5 px-4 py-1.5 border-t border-base-200/50">
            {#each headerIssues as issue}
              <span class="text-xs text-caution">
                {issue.message}
                {#if issue.docs_url}
                  <button onclick={() => openUrl(issue.docs_url!)} class="underline ml-1">Get one</button>
                {/if}
              </span>
            {/each}
          </div>
        {/if}
        {#if isExpanded}
          <div class="px-4 py-2 space-y-1 {!isOn ? 'opacity-40 pointer-events-none' : ''}">
            {#each contentChildren as item}
              {#if item.kind === 'leaf'}
                <div class="py-1.5 border-b border-base-200/50 last:border-b-0">
                  {@render leafControl(item)}
                </div>
              {:else if item.kind === 'group'}
                <div class="mt-2 mb-1 ml-1">
                  <Self group={item} depth={depth + 1} />
                </div>
              {/if}
            {/each}
          </div>
        {/if}
      </div>
    {:else}
      <!-- Non-toggle subgroup; only depth-0 children get data-subgroup for scroll tracking -->
      <div id="settings-group-{child.name}" data-subgroup={depth === 0 ? child.name : null} class="mt-6 first:mt-0 mb-2 scroll-mt-4">
        <h2 class="text-lg font-semibold text-interactive mb-0.5">{child.name}</h2>
        {#if child.description}
          <p class="text-xs text-base-content/50 mb-2">{child.description}</p>
        {/if}
        <div class="space-y-1">
          <Self group={child} depth={depth + 1} />
        </div>
      </div>
    {/if}
  {/if}
{/each}

