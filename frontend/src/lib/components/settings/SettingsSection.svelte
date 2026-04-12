<script lang="ts">
  import { slide } from 'svelte/transition';
  import type { SettingsGroup, SettingsLeaf, SettingsAction, SettingsNode, SettingValue } from '../../types/settings';
  import { settingsStore } from '../../stores/settings.svelte.ts';
  import { themeStore } from '../../stores/theme.svelte.ts';
  import { Widget, SideEffect, ActionKind } from '../../models/settings-enums';
  import Self from './SettingsSection.svelte';
  import PresetSection from './PresetSection.svelte';
  import ToggleControl from './widgets/ToggleControl.svelte';
  import TextControl from './widgets/TextControl.svelte';
  import NumberControl from './widgets/NumberControl.svelte';
  import PasswordControl from './widgets/PasswordControl.svelte';
  import SelectControl from './widgets/SelectControl.svelte';
  import FileEditorControl from './widgets/FileEditorControl.svelte';
  import DomainChipsControl from './widgets/DomainChipsControl.svelte';
  import CaretDown from 'phosphor-svelte/lib/CaretDown';
  import WarningCircle from 'phosphor-svelte/lib/WarningCircle';

  let { group, depth = 0 }: { group: SettingsGroup; depth?: number } = $props();

  // Track collapsed state for toggle-gated groups.
  let expandedGroups = $state<Set<string>>(new Set());

  function toggleGroup(key: string) {
    const next = new Set(expandedGroups);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    expandedGroups = next;
  }

  // Find the toggle leaf within a group's children (the one matching enabled_by).
  function findToggle(children: SettingsNode[], enabledBy: string | null | undefined): SettingsLeaf | null {
    if (!enabledBy) return null;
    for (const child of children) {
      if (child.kind === 'leaf' && child.id === enabledBy) return child;
    }
    return null;
  }

  function isToggleOn(children: SettingsNode[], enabledBy: string | null | undefined): boolean {
    const toggle = findToggle(children, enabledBy);
    if (!toggle) return true;
    return toggle.effective_value === true;
  }

  async function handleUpdate(id: string, value: unknown) {
    const leaf = settingsStore.findLeaf(id);
    if (leaf?.metadata.side_effect === SideEffect.ToggleTheme) {
      themeStore.toggleMode();
    }
    if (leaf?.setting_type === 'bool') {
      await settingsStore.updateImmediate(id, value as SettingValue);
    } else {
      settingsStore.stage(id, value as SettingValue);
    }
  }

  /** Collect all lint issues for all leaves inside a group (recursive). */
  function groupIssues(children: SettingsNode[]): { id: string; severity: string; message: string; docs_url?: string | null }[] {
    const issues: { id: string; severity: string; message: string; docs_url?: string | null }[] = [];
    for (const child of children) {
      if (child.kind === 'leaf') {
        issues.push(...settingsStore.issuesFor(child.id));
      } else if (child.kind === 'group') {
        issues.push(...groupIssues(child.children));
      }
    }
    return issues;
  }

  /** Check if a provider has any required API key fields that are empty. */
  function hasMissingApiKey(children: SettingsNode[]): boolean {
    for (const child of children) {
      if (child.kind === 'leaf' && child.setting_type === 'apikey') {
        const val = child.effective_value;
        if (typeof val === 'string' && val.length === 0) return true;
      } else if (child.kind === 'group') {
        if (hasMissingApiKey(child.children)) return true;
      }
    }
    return false;
  }

  function resolveWidget(leaf: SettingsLeaf): Widget {
    return settingsStore.model?.getWidget(leaf) ?? Widget.TextInput;
  }
</script>

{#snippet leafControl(s: SettingsLeaf)}
  {@const widget = resolveWidget(s)}
  {@const disabled = s.corp_locked || !s.enabled}
  {@const issues = settingsStore.issuesFor(s.id)}
  {#if widget === Widget.Toggle}
    <ToggleControl leaf={s} {disabled} onchange={(v) => handleUpdate(s.id, v)} />
  {:else if widget === Widget.FileEditor}
    <FileEditorControl leaf={s} {disabled} onchange={(v) => handleUpdate(s.id, v)} />
  {:else if widget === Widget.DomainChips || widget === Widget.StringChips}
    <DomainChipsControl leaf={s} {disabled} onchange={(v) => handleUpdate(s.id, v)} />
  {:else if widget === Widget.PasswordInput}
    <PasswordControl leaf={s} {disabled} onchange={(v) => handleUpdate(s.id, v)} />
  {:else if widget === Widget.NumberInput}
    <NumberControl leaf={s} {disabled} onchange={(v) => handleUpdate(s.id, v)} />
  {:else if widget === Widget.Select}
    <SelectControl leaf={s} {disabled} onchange={(v) => handleUpdate(s.id, v)} />
  {:else}
    <TextControl leaf={s} {disabled} onchange={(v) => handleUpdate(s.id, v)} />
  {/if}
  {#if issues.length > 0}
    <div class="flex flex-col gap-y-0.5 pb-1">
      {#each issues as issue}
        <span class="text-xs {issue.severity === 'error' ? 'text-red-700 dark:text-red-300' : 'text-amber-700 dark:text-amber-400'}">
          {issue.message}
          {#if issue.docs_url}
            <a href={issue.docs_url} target="_blank" rel="noopener" class="underline ml-1">Get one</a>
          {/if}
        </span>
      {/each}
    </div>
  {/if}
{/snippet}

{#snippet actionControl(a: SettingsAction)}
  {#if a.action === ActionKind.PresetSelect}
    <div class="mt-4 first:mt-0 mb-2">
      <h3 class="text-base font-semibold text-primary mb-1">{a.name}</h3>
      {#if a.description}
        <p class="text-xs text-muted-foreground-1 mb-2">{a.description}</p>
      {/if}
      <PresetSection />
    </div>
  {:else if a.action === ActionKind.CheckUpdate}
    <div class="flex items-center justify-between py-3">
      <div>
        <span class="text-sm font-medium text-foreground">{a.name}</span>
        {#if a.description}
          <p class="text-xs text-muted-foreground-1 mt-0.5">{a.description}</p>
        {/if}
      </div>
      <button
        type="button"
        class="py-2 px-4 text-sm font-medium rounded-lg border border-line-2 bg-layer text-foreground hover:bg-layer-hover transition-colors"
      >
        Check now
      </button>
    </div>
  {/if}
{/snippet}

<!-- Top-level group header (depth 0) -->
{#if depth === 0}
  <div class="mb-6">
    <h2 class="text-xl font-bold text-foreground">{group.name}</h2>
    {#if group.description}
      <p class="text-sm text-muted-foreground-1 mt-0.5">{group.description}</p>
    {/if}
  </div>
{/if}

<!-- At depth 0, wrap consecutive leaf/action items in a card (matching Appearance pattern) -->
{#if depth === 0}
  {@const topLevelItems = group.children.filter(c => c.kind === 'leaf' || c.kind === 'action')}
  {#if topLevelItems.length > 0}
    <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider mb-6">
      {#each topLevelItems as item}
        {#if item.kind === 'leaf'}
          <div class="px-4">
            {@render leafControl(item)}
          </div>
        {:else if item.kind === 'action'}
          <div class="px-4 py-2">
            {@render actionControl(item)}
          </div>
        {/if}
      {/each}
    </div>
  {/if}
{/if}

<!-- Render children (groups at depth 0, everything at depth > 0) -->
{#each group.children as child (child.kind === 'leaf' ? child.id : child.kind === 'group' ? child.key : child.kind === 'action' ? child.key : child.kind === 'mcp_server' ? child.key : Math.random())}
  {#if depth > 0 && child.kind === 'action'}
    {@render actionControl(child)}
  {:else if depth > 0 && child.kind === 'leaf'}
    <div class="border-b border-card-divider last:border-b-0">
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
      {@const missingKey = isOn && hasMissingApiKey(child.children)}
      <!-- Toggle-gated group: polished card with darker header -->
      <div class="bg-card border border-card-line rounded-xl overflow-hidden mb-3">
        <!-- Header: bg-background-1 = slightly darker, matching Appearance section headers -->
        <div class="flex items-center gap-x-3 px-4 py-3 bg-background-1">
          {#if toggle}
            <button
              type="button"
              class="relative inline-flex shrink-0 h-5 w-9 border-2 border-transparent rounded-full cursor-pointer transition-colors duration-200
                {toggle.effective_value === true ? 'bg-primary' : 'bg-surface-2'}
                {toggle.corp_locked ? 'opacity-50 cursor-not-allowed' : ''}"
              role="switch"
              aria-checked={toggle.effective_value === true}
              aria-label="Toggle {child.name}"
              disabled={toggle.corp_locked}
              onclick={(e) => { e.stopPropagation(); handleUpdate(toggle.id, !(toggle.effective_value === true)); }}
            >
              <span
                class="pointer-events-none inline-block size-4 rounded-full bg-white shadow-sm ring-0 transition-transform duration-200
                  {toggle.effective_value === true ? 'translate-x-4' : 'translate-x-0'}"
              ></span>
            </button>
          {/if}
          <button
            type="button"
            class="flex-1 text-left min-w-0"
            onclick={() => toggleGroup(child.key)}
          >
            <span class="text-sm font-semibold text-foreground">{child.name}</span>
            {#if toggle?.corp_locked}
              <span class="text-[10px] px-1.5 py-0.5 rounded-full bg-destructive/10 text-destructive font-medium ml-1.5">corp</span>
            {/if}
            {#if child.description}
              <span class="text-xs text-muted-foreground-1 ml-2">{child.description}</span>
            {/if}
          </button>
          <!-- Warning badge when collapsed and issues/missing key -->
          {#if !isExpanded && (headerIssues.length > 0 || missingKey)}
            <span class="text-amber-700 dark:text-amber-400" title="{headerIssues.length} issue{headerIssues.length === 1 ? '' : 's'}">
              <WarningCircle size={18} weight="fill" />
            </span>
          {/if}
          <button
            type="button"
            class="p-1.5 rounded-md text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover transition-colors"
            aria-label={isExpanded ? 'Collapse' : 'Expand'}
            onclick={() => toggleGroup(child.key)}
          >
            <CaretDown size={14} class="transition-transform duration-300 {isExpanded ? 'rotate-180' : ''}" />
          </button>
        </div>
        <!-- Collapsed issue summary -->
        {#if !isExpanded && headerIssues.length > 0}
          <div class="px-4 py-2 border-t border-card-divider bg-amber-50/50 dark:bg-amber-900/10">
            {#each headerIssues as issue}
              <p class="text-xs text-yellow-700 dark:text-yellow-400">
                {issue.message}
                {#if issue.docs_url}
                  <a href={issue.docs_url} target="_blank" rel="noopener" class="underline ml-1">Get one</a>
                {/if}
              </p>
            {/each}
          </div>
        {/if}
        <!-- Collapsible content -->
        {#if isExpanded}
          <div
            transition:slide={{ duration: 300 }}
            class="divide-y divide-card-divider {!isOn ? 'opacity-40 pointer-events-none' : ''}"
          >
            {#each contentChildren as item}
              {#if item.kind === 'leaf'}
                <div class="px-4">
                  {@render leafControl(item)}
                </div>
              {:else if item.kind === 'group'}
                <!-- Nested subgroup: section label like Appearance's "Interface"/"Terminal" -->
                <div class="px-4 pt-4 pb-2">
                  <h4 class="text-xs font-semibold text-muted-foreground-1 uppercase tracking-wider mb-1">{item.name}</h4>
                  {#if item.description}
                    <p class="text-xs text-muted-foreground-1 mb-2">{item.description}</p>
                  {/if}
                  <Self group={item} depth={depth + 1} />
                </div>
              {/if}
            {/each}
          </div>
        {/if}
      </div>
    {:else}
      <!-- Non-toggle subgroup: section label + card, matching Appearance pattern -->
      <h3 class="text-xs font-semibold text-muted-foreground-1 uppercase tracking-wider mb-2 mt-6 first:mt-0">{child.name}</h3>
      {#if child.description}
        <p class="text-xs text-muted-foreground-1 mb-2">{child.description}</p>
      {/if}
      <div class="bg-card border border-card-line rounded-xl divide-y divide-card-divider mb-6">
        <Self group={child} depth={depth + 1} />
      </div>
    {/if}
  {/if}
{/each}
