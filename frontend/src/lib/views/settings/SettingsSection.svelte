<script lang="ts">
  import type { SettingsGroup, SettingsLeaf, SettingsNode, SettingValue } from '../../types';
  import { settingsStore } from '../../stores/settings.svelte';
  import { themeStore } from '../../stores/theme.svelte';
  import Self from './SettingsSection.svelte';

  let { group, depth = 0 }: { group: SettingsGroup; depth?: number } = $props();

  /** Extract path + content from a File setting value. */
  function fileValue(v: SettingValue): { path: string; content: string } {
    if (typeof v === 'object' && v !== null && 'path' in v) return v as { path: string; content: string };
    console.error('fileValue: expected { path, content } but got', typeof v, v);
    return { path: '', content: String(v) };
  }

  // Track collapsed state for sub-groups with enabled_by toggle.
  let expandedGroups = $state<Set<string>>(new Set());
  // Track collapsed state for "advanced" (collapsed) settings within a group.
  let showAdvanced = $state<Set<string>>(new Set());
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

  function toggleGroup(key: string) {
    const next = new Set(expandedGroups);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    expandedGroups = next;
  }

  function toggleAdvanced(key: string) {
    const next = new Set(showAdvanced);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    showAdvanced = next;
  }

  function toggleReveal(id: string) {
    const next = new Set(revealedKeys);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    revealedKeys = next;
  }

  function startEditing(id: string, fv: { path: string; content: string }) {
    const next = new Set(editingFiles);
    next.add(id);
    editingFiles = next;
    const drafts = new Map(fileDrafts);
    drafts.set(id, formatJson(fv.content));
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

  async function saveFile(id: string) {
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
    // Compact the JSON for storage (remove pretty-print whitespace).
    let compacted = draft.trim();
    try {
      compacted = JSON.stringify(JSON.parse(compacted));
    } catch { /* save as-is if not valid JSON */ }
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

  // Separate children into core (non-collapsed) and advanced (collapsed).
  function partitionChildren(children: SettingsNode[]): { core: SettingsNode[]; advanced: SettingsNode[] } {
    const core: SettingsNode[] = [];
    const advanced: SettingsNode[] = [];
    for (const child of children) {
      if (child.kind === 'leaf' && child.collapsed) {
        advanced.push(child);
      } else {
        core.push(child);
      }
    }
    return { core, advanced };
  }

  async function handleUpdate(id: string, value: unknown) {
    // Theme special case
    if (id === 'appearance.dark_mode') {
      themeStore.toggle();
    }
    await settingsStore.update(id, value as any);
  }

  function formatJson(text: string): string {
    try {
      return JSON.stringify(JSON.parse(text), null, 2);
    } catch {
      return text;
    }
  }

  /** Simple JSON syntax highlighter -- returns HTML with colored spans. */
  function highlightJson(text: string): string {
    const formatted = formatJson(text);
    // Escape HTML first
    const escaped = formatted
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
    // Apply token coloring
    return escaped
      // Strings (keys and values)
      .replace(
        /("(?:[^"\\]|\\.)*")(\s*:)?/g,
        (match, str, colon) => {
          if (colon) {
            // It's a key
            return `<span class="json-key">${str}</span>${colon}`;
          }
          // It's a string value
          return `<span class="json-string">${str}</span>`;
        },
      )
      // Booleans and null
      .replace(/\b(true|false|null)\b/g, '<span class="json-bool">$1</span>')
      // Numbers
      .replace(/\b(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)\b/g, '<span class="json-number">$1</span>');
  }
</script>

<style>
  :global(.json-key) {
    color: oklch(0.75 0.15 250); /* blue */
  }
  :global(.json-string) {
    color: oklch(0.72 0.14 150); /* green-teal */
  }
  :global(.json-bool) {
    color: oklch(0.7 0.16 300); /* purple */
  }
  :global(.json-number) {
    color: oklch(0.75 0.14 60); /* orange */
  }
</style>

{#snippet fileControl(s: SettingsLeaf)}
  {@const issues = settingsStore.issuesFor(s.id)}
  {@const disabled = s.corp_locked || !s.enabled}
  {@const isEditing = editingFiles.has(s.id)}
  {@const fv = fileValue(s.effective_value)}
  <div class="rounded-md border border-base-300 bg-base-200/30 overflow-hidden">
    <!-- File header bar -->
    <div class="flex items-center gap-2 px-3 py-1.5 border-b border-base-300/50 bg-base-200/50">
      <svg class="size-3.5 text-base-content/40 flex-shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>
      <span class="text-xs font-medium flex-1 min-w-0 truncate">{s.name}</span>
      {#if fv.path || isEditing}
        {#if isEditing}
          <input
            type="text"
            class="text-[10px] font-mono text-base-content/50 bg-transparent border-b border-base-content/20 focus:outline-none focus:border-primary w-48 text-right px-0.5"
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
        <span class="badge badge-xs badge-secondary">corp</span>
      {/if}
      {#if s.source === 'user'}
        <span class="badge badge-xs badge-info badge-outline">modified</span>
      {/if}
      <!-- Copy button -->
      <button
        class="btn btn-ghost btn-xs text-base-content/40"
        onclick={() => copyContent(isEditing ? (fileDrafts.get(s.id) ?? fv.content) : fv.content, s.id)}
        title="Copy to clipboard"
      >
        {#if copiedId === s.id}
          <svg class="size-3.5 text-success" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="20 6 9 17 4 12" /></svg>
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
            class="btn btn-primary btn-xs"
            onclick={() => saveFile(s.id)}
          >Save</button>
        {:else}
          <button
            class="btn btn-ghost btn-xs text-base-content/50"
            onclick={() => startEditing(s.id, fv)}
          >Edit</button>
        {/if}
      {/if}
    </div>
    <!-- File content -->
    {#if isEditing}
      <textarea
        class="w-full bg-transparent font-mono text-xs leading-relaxed p-3 focus:outline-none resize-y min-h-20"
        rows={Math.min(Math.max(formatJson(fv.content).split('\n').length + 1, 4), 20)}
        value={fileDrafts.get(s.id) ?? formatJson(fv.content)}
        oninput={(e) => {
          const drafts = new Map(fileDrafts);
          drafts.set(s.id, e.currentTarget.value);
          fileDrafts = drafts;
        }}
      ></textarea>
    {:else}
      <pre class="font-mono text-xs leading-relaxed p-3 overflow-x-auto whitespace-pre-wrap">{@html highlightJson(fv.content)}</pre>
    {/if}
    <!-- Lint issues -->
    {#if issues.length > 0}
      <div class="px-3 pb-2 flex flex-col gap-0.5">
        {#each issues as issue}
          <span class="text-xs {issue.severity === 'error' ? 'text-error' : 'text-warning'}">
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
  {:else}
  <div class="form-control">
    <div class="flex items-start gap-3">
      <div class="flex-1 min-w-0">
        <div class="flex items-center gap-2 mb-0.5">
          <span class="text-sm font-medium">{s.name}</span>
          {#if s.corp_locked}
            <span class="badge badge-xs badge-secondary">corp</span>
          {/if}
          {#if s.source === 'user'}
            <span class="badge badge-xs badge-info badge-outline">modified</span>
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
            class="toggle toggle-sm toggle-primary"
            checked={s.effective_value === true}
            {disabled}
            onchange={(e) => handleUpdate(s.id, e.currentTarget.checked)}
          />
        {:else if s.setting_type === 'apikey' || s.setting_type === 'password'}
          <div class="flex items-center gap-1">
            <input
              type={revealedKeys.has(s.id) ? 'text' : 'password'}
              class="input input-sm input-bordered w-64 font-mono text-xs"
              value={String(s.effective_value)}
              {disabled}
              placeholder={s.setting_type === 'apikey' ? 'sk-...' : ''}
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
          <span class="text-xs {issue.severity === 'error' ? 'text-error' : 'text-warning'}">
            {issue.message}
          </span>
        {/each}
      </div>
    {/if}
  </div>
  {/if}
{/snippet}

<!-- Top-level group header -->
{#if depth === 0}
  <div class="mb-4">
    <h2 class="text-lg font-semibold">{group.name}</h2>
    {#if group.description}
      <p class="text-sm text-base-content/50">{group.description}</p>
    {/if}
  </div>
{/if}

<!-- Render children -->
{#each group.children as child}
  {#if child.kind === 'leaf'}
    <div class="py-2 border-b border-base-200 last:border-b-0">
      {@render leafControl(child)}
    </div>
  {:else if child.kind === 'group'}
    {@const hasToggle = !!child.enabled_by}
    {@const toggle = findToggle(child.children, child.enabled_by)}
    {@const isOn = isToggleOn(child.children, child.enabled_by)}
    {@const { core, advanced } = partitionChildren(child.children.filter(c => !(c.kind === 'leaf' && c.id === child.enabled_by)))}
    {@const isExpanded = expandedGroups.has(child.key) || !hasToggle}
    {@const showAdv = showAdvanced.has(child.key)}

    <div class="rounded-lg border border-base-300 mb-3 overflow-hidden">
      <!-- Group header -->
      <div class="flex items-center gap-3 px-4 py-2.5 bg-base-200/40">
        {#if hasToggle && toggle}
          <input
            type="checkbox"
            class="toggle toggle-sm toggle-primary"
            checked={toggle.effective_value === true}
            disabled={toggle.corp_locked}
            onchange={(e) => handleUpdate(toggle.id, e.currentTarget.checked)}
          />
        {/if}
        <button
          class="flex-1 text-left"
          onclick={() => { if (hasToggle) toggleGroup(child.key); }}
        >
          <span class="text-sm font-medium">{child.name}</span>
          {#if toggle?.corp_locked}
            <span class="badge badge-xs badge-secondary ml-1">corp</span>
          {/if}
          {#if child.description}
            <span class="text-xs text-base-content/40 ml-2">{child.description}</span>
          {/if}
        </button>
        {#if hasToggle}
          <button
            class="btn btn-ghost btn-xs"
            aria-label={isExpanded ? 'Collapse' : 'Expand'}
            onclick={() => toggleGroup(child.key)}
          >
            <svg class="size-3 transition-transform {isExpanded ? 'rotate-180' : ''}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="6 9 12 15 18 9" /></svg>
          </button>
        {/if}
      </div>

      <!-- Group content -->
      {#if isExpanded}
        <div class="px-4 py-2 space-y-1 {hasToggle && !isOn ? 'opacity-40 pointer-events-none' : ''}">
          {#each core as item}
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

          <!-- Advanced (collapsed) settings -->
          {#if advanced.length > 0}
            <div class="pt-1">
              <button
                class="btn btn-ghost btn-xs text-base-content/40"
                onclick={() => toggleAdvanced(child.key)}
              >
                {showAdv ? 'Hide' : 'Show'} {advanced.length} advanced {advanced.length === 1 ? 'setting' : 'settings'}
                <svg class="size-3 ml-0.5 transition-transform {showAdv ? 'rotate-180' : ''}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="6 9 12 15 18 9" /></svg>
              </button>
              {#if showAdv}
                <div class="mt-1 space-y-1 border-t border-base-200/50 pt-1">
                  {#each advanced as adv}
                    {#if adv.kind === 'leaf'}
                      <div class="py-1.5 border-b border-base-200/50 last:border-b-0">
                        {@render leafControl(adv)}
                      </div>
                    {/if}
                  {/each}
                </div>
              {/if}
            </div>
          {/if}
        </div>
      {/if}
    </div>
  {/if}
{/each}
