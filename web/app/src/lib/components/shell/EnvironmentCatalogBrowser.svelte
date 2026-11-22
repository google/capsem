<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import {
    CatalogSourceOrigin, type CatalogEntryMetadata, type CatalogSnapshot, type CatalogStatus,
    type CatalogRuleKind, type EnvironmentCatalogPin, type RuleCatalogDetailResponse, type SkillCatalogDetailResponse,
  } from '@capsem/sdk';
  import { environmentCatalogStatus, getPublishedRule, getPublishedSkill, listEnvironmentCatalog } from '../../api';
  import { ApiError } from '../../gateway-sdk';
  import { catalogStatusText, currentCatalogSelection, stageCatalogSelection } from '../../models/environment-catalog';
  import type { EnvironmentFields } from '../../models/environment-editor';
  import { sessionActionError } from '../../models/session-action-error';

  type Row = { metadata: CatalogEntryMetadata; ruleKind?: CatalogRuleKind };

  let { kind, sections = $bindable() }: {
    kind: 'skills' | 'rules'; sections: EnvironmentFields['sections'];
  } = $props();
  const id = $props.id();
  const button = 'rounded-md border border-line-2 px-2.5 py-1.5 text-xs font-medium hover:bg-muted-hover';
  const itemName = $derived(kind === 'skills' ? 'skill' : 'rule');
  let status = $state<CatalogStatus | undefined>();
  let catalog = $state<CatalogSnapshot | null>(null);
  let entries = $state<Row[]>([]);
  let cursor = $state<string | null>(null);
  let search = $state('');
  let submittedSearch = $state('');
  let busy = $state(false);
  let inspecting = $state<string | null>(null);
  let detail = $state<SkillCatalogDetailResponse | RuleCatalogDetailResponse | null>(null);
  let selectedFile = $state('SKILL.md');
  let problem = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let listRequest: AbortController | undefined;
  let detailRequest: AbortController | undefined;
  let detailTitle = $state<HTMLElement>();
  let detailTrigger: HTMLButtonElement | undefined;
  const rowKey = (row: Row) => `${row.metadata.id}:${row.metadata.version}`;
  const target = $derived(detail && 'definition' in detail ? detail.definition.kind : 'skills');
  const current = $derived(detail ? currentCatalogSelection(sections[target], target, detail.selection.id) : undefined);
  const sameSelection = $derived(!!detail && current?.version === detail.selection.version
    && (!current.expected_catalog || (current.expected_catalog.reference === detail.selection.catalog_reference
      && current.expected_catalog.digest === detail.selection.resolved_digest)));

  function clearDetail() {
    detailRequest?.abort();
    inspecting = null;
    detail = null;
  }

  function report(error: unknown) {
    if (error instanceof ApiError && error.status === 409) {
      catalog = null;
      entries = [];
      cursor = null;
      clearDetail();
      problem = 'The catalog changed. Search again to browse the current snapshot.';
    } else {
      problem = sessionActionError(error);
    }
  }

  async function readStatus(signal: AbortSignal) {
    try {
      const all = await environmentCatalogStatus(signal);
      if (!signal.aborted) status = all.find(item => item.kind === kind);
    } catch {
      if (!signal.aborted) status = undefined;
    }
  }

  async function load(after: string | undefined = undefined) {
    const snapshot = catalog;
    listRequest?.abort();
    clearDetail();
    const controller = new AbortController();
    const signal = controller.signal;
    listRequest = controller;
    busy = true;
    problem = null;
    notice = null;
    if (!after) {
      submittedSearch = search.trim();
      catalog = null;
      cursor = null;
      entries = [];
    }
    try {
      const result = await listEnvironmentCatalog(kind, { signal, search: submittedSearch, ...(after ? { after } : {}) });
      if (signal.aborted) return;
      if (after && (result.catalog.snapshot !== snapshot?.snapshot || result.next_cursor === after)) {
        report(new ApiError(409, 'catalog snapshot changed'));
        return;
      }
      const page = 'skills' in result ? result.skills.map(metadata => ({ metadata }))
        : result.rules.map(rule => ({ metadata: rule.metadata, ruleKind: rule.kind }));
      catalog = result.catalog;
      entries = after ? [...entries, ...page] : page;
      cursor = result.next_cursor ?? null;
    } catch (error) {
      if (!signal.aborted) report(error);
    } finally {
      if (!signal.aborted) {
        busy = false;
        void readStatus(signal);
      }
    }
  }

  async function inspect(row: Row, trigger: HTMLButtonElement) {
    const snapshot = catalog;
    if (!snapshot) return;
    clearDetail();
    detailTrigger = trigger;
    problem = null;
    notice = null;
    inspecting = rowKey(row);
    const controller = new AbortController();
    const signal = controller.signal;
    detailRequest = controller;
    const { id: entryId, version } = row.metadata;
    try {
      const result = kind === 'skills' ? await getPublishedSkill(entryId, version, snapshot, signal)
        : await getPublishedRule(entryId, version, snapshot, signal);
      if (signal.aborted) return;
      detail = result;
      selectedFile = 'SKILL.md';
      await tick();
      if (!signal.aborted) detailTitle?.focus();
    } catch (error) {
      if (!signal.aborted) {
        report(error);
        if (listRequest) void readStatus(listRequest.signal);
      }
    } finally {
      if (!signal.aborted) inspecting = null;
    }
  }

  function stage(pin: EnvironmentCatalogPin) {
    try {
      sections[target] = stageCatalogSelection(sections[target], target, pin);
      notice = `Added ${detail?.metadata.name ?? pin.id} ${pin.version} to the draft.`;
      problem = null;
    } catch (error) {
      problem = sessionActionError(error);
    }
  }

  function closeDetail() {
    clearDetail();
    detailTrigger?.focus();
  }

  function toggle(event: Event & { currentTarget: EventTarget & HTMLDetailsElement }) {
    if (event.currentTarget.open) void load();
    else {
      listRequest?.abort();
      clearDetail();
    }
  }

  onDestroy(() => {
    listRequest?.abort();
    detailRequest?.abort();
  });
</script>

<details class="min-w-0 rounded-lg border border-line-2" ontoggle={toggle}>
  <summary class="cursor-pointer px-3 py-2 text-sm font-medium">Browse {itemName} catalog</summary>
  <section aria-label={`${itemName === 'skill' ? 'Skill' : 'Rule'} catalog`} class="min-w-0 space-y-3 p-3 pt-0">
    {#if status}
      {@const display = catalogStatusText(status)}
      <div class="space-y-1 rounded-md bg-muted/60 px-3 py-2 text-xs">
        <p class="font-medium">{display.availability}{status.origin ? ` · ${status.origin === CatalogSourceOrigin.CORPORATE ? 'Corporate configuration' : 'Release manifest'}` : ''}</p>
        {#if status.reference}<p class="break-all font-mono">{status.reference}</p>{/if}
        {#if status.digest}<p class="break-all text-muted-foreground-1">Last verified digest {status.digest}</p>{/if}
        {#if status.last_successful_refresh !== null}
          {@const refreshed = new Date(status.last_successful_refresh)}
          <p class="text-muted-foreground-1">Refreshed <time datetime={refreshed.toISOString()}>{refreshed.toLocaleString()}</time></p>
        {/if}
        {#if display.error}<p class="text-muted-foreground-1">{display.error}</p>{/if}
      </div>
    {/if}
    <form class="flex flex-wrap items-end gap-2" onsubmit={event => { event.preventDefault(); void load(); }}>
      <label class="min-w-0 flex-1 space-y-1">
        <span class="text-xs font-medium">Search {kind}</span>
        <input type="search" bind:value={search} maxlength="64" class="w-full rounded-md border border-line-2 bg-background-1 px-2.5 py-1.5 text-sm" />
      </label>
      <button type="submit" class={button}>Search</button>
    </form>
    {#if problem}<p role="alert" class="text-xs text-destructive-text">{problem}</p>{/if}
    {#if busy}<p role="status" class="text-xs text-muted-foreground-1">Loading {kind}…</p>{/if}
    {#if catalog && entries.length === 0 && !busy}<p class="text-xs text-muted-foreground-1">No {kind} match this search.</p>{/if}
    {#if entries.length}
      <ul class="divide-y divide-line-2 rounded-md border border-line-2">
        {#each entries as entry (rowKey(entry))}
          <li class="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 px-3 py-2">
            <div class="min-w-0 flex-1 space-y-0.5">
              <p class="break-words text-sm font-medium">{entry.metadata.name}</p>
              <p class="break-words text-xs text-muted-foreground-1">{entry.metadata.id} · {entry.metadata.version}{entry.ruleKind ? ` · ${entry.ruleKind === 'access' ? 'Access' : 'Detection'}` : ''}{entry.metadata.status === 'deprecated' ? ' · Deprecated' : ''}</p>
            </div>
            <button type="button" class={button} aria-label={`Inspect ${entry.metadata.name} ${entry.metadata.version}`}
              onclick={event => inspect(entry, event.currentTarget)}>
              {inspecting === rowKey(entry) ? 'Opening…' : 'Inspect'}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
    {#if cursor}
      <button type="button" class={button} disabled={busy} onclick={() => { if (cursor) void load(cursor); }}>More {kind}</button>
    {/if}
    {#if detail}
      <section aria-labelledby={`${id}-entry`} class="min-w-0 space-y-3 rounded-md border border-line-2 p-3">
        <div class="flex flex-wrap items-start justify-between gap-2">
          <div class="min-w-0">
            <h3 bind:this={detailTitle} tabindex="-1" id={`${id}-entry`} class="break-words text-sm font-medium">{detail.metadata.name} {detail.metadata.version}</h3>
            <p class="break-words text-xs text-muted-foreground-1">{detail.metadata.publisher} · {detail.metadata.license}</p>
          </div>
          <button type="button" class={button} onclick={closeDetail}>Close details</button>
        </div>
        {#if detail.metadata.description}<p class="whitespace-pre-wrap break-words text-xs">{detail.metadata.description}</p>{/if}
        {#if detail.metadata.status_reason}<p class="break-words text-xs text-muted-foreground-1">{detail.metadata.status_reason}</p>{/if}
        {#if 'files' in detail}
          <div class="space-y-1">
            <label for={`${id}-file`} class="block text-xs font-medium">Skill file</label>
            <select id={`${id}-file`} bind:value={selectedFile} class="block max-w-full rounded-md border border-line-2 bg-background-1 p-1.5 text-xs">
              {#each Object.keys(detail.files).sort() as path (path)}<option value={path}>{path}</option>{/each}
            </select>
          </div>
          <textarea readonly aria-label="Skill file contents" value={detail.files[selectedFile]} rows="8"
            class="block w-full resize-y rounded-md bg-muted p-2 font-mono text-xs"></textarea>
        {:else}
          <textarea readonly aria-label="Rule definition" value={JSON.stringify(detail.definition, null, 2)} rows="8"
            class="block w-full resize-y rounded-md bg-muted p-2 font-mono text-xs"></textarea>
          {#if Object.keys(detail.parameters).length}
            <dl aria-label="Rule parameters" class="space-y-1 text-xs">
              {#each Object.entries(detail.parameters) as [name, parameter] (name)}
                <div class="grid grid-cols-[minmax(5rem,1fr)_minmax(0,2fr)] gap-2">
                  <dt class="break-words font-medium">{name}</dt>
                  <dd class="break-words">{parameter.type} · Default {JSON.stringify(parameter.default)}
                    {#if parameter.type === 'integer'} · {parameter.min} to {parameter.max}
                    {:else if parameter.type === 'choice'} · Choices {parameter.choices.join(', ')}
                    {:else if parameter.type === 'text'} · Up to {parameter.max_bytes} bytes{/if}
                  </dd>
                </div>
              {/each}
            </dl>
          {/if}
        {/if}
        {#if current && current.version !== detail.selection.version}
          <p class="text-xs text-muted-foreground-1">Draft version {current.version}{target !== 'skills' ? '. Replacing it uses the published parameter defaults.' : ''}</p>
        {/if}
        <button type="button" class="rounded-md bg-primary px-2.5 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary-hover disabled:opacity-50"
          disabled={sameSelection} onclick={() => { if (detail) stage(detail.selection); }}>
          {sameSelection ? 'Selected in draft' : current ? 'Update draft selection' : `Add ${itemName} to draft`}
        </button>
      </section>
    {/if}
    {#if notice}<p role="status" class="text-xs text-primary">{notice}</p>{/if}
  </section>
</details>
