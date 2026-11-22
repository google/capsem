<script lang="ts">
  import { onMount } from 'svelte';
  import { RequestReviewAvailability, type EnvironmentRevisionDetailResponse, type EnvironmentRevisionReference,
    type EnvironmentRevisionSummary } from '@capsem/sdk';
  import { getSavedEnvironmentRevision, listSavedEnvironmentRevisions } from '../../api';
  import { sessionActionError } from '../../models/session-action-error';
  import EnvironmentPromotion from './EnvironmentPromotion.svelte';
  import EnvironmentApplication from './EnvironmentApplication.svelte';

  let { reference }: { reference: EnvironmentRevisionReference } = $props();
  let rows = $state<EnvironmentRevisionSummary[]>([]);
  let cursor = $state<number | null>(null);
  let selected = $state<EnvironmentRevisionReference | null>(null);
  let detail = $state<EnvironmentRevisionDetailResponse | null>(null);
  let loading = $state(false);
  let reading = $state(false);
  let pageError = $state<string | null>(null);
  let detailError = $state<string | null>(null);
  const pageReader = new AbortController();
  let detailReader: AbortController | null = null;

  async function loadPage(before: number | null = null) {
    loading = true;
    pageError = null;
    try {
      const page = await listSavedEnvironmentRevisions(reference.environment_id, {
        ...(before != null ? { before } : {}), signal: pageReader.signal,
      });
      if (pageReader.signal.aborted) return;
      rows = before == null ? page.revisions : [...rows, ...page.revisions];
      cursor = page.next_cursor ?? null;
    } catch (err: unknown) {
      if (!pageReader.signal.aborted) pageError = sessionActionError(err);
    } finally {
      if (!pageReader.signal.aborted) loading = false;
    }
  }

  async function inspect(revision: EnvironmentRevisionReference) {
    detailReader?.abort();
    const controller = new AbortController();
    detailReader = controller;
    selected = revision;
    detail = null;
    detailError = null;
    reading = true;
    try {
      const result = await getSavedEnvironmentRevision(revision, controller.signal);
      if (!controller.signal.aborted) detail = result;
    } catch (err: unknown) {
      if (!controller.signal.aborted) detailError = sessionActionError(err);
    } finally {
      if (!controller.signal.aborted) reading = false;
    }
  }

  onMount(() => {
    void loadPage();
    return () => { pageReader.abort(); detailReader?.abort(); };
  });
</script>

<section aria-label="Change history" class="space-y-3 text-sm">
  {#if loading}<p role="status" class="text-muted-foreground-1">Loading revisions…</p>{/if}
  {#if pageError}
    <p role="alert" class="text-destructive-text">{pageError}</p>
    <button type="button" class="text-primary hover:underline" onclick={() => loadPage(cursor)}>Retry history</button>
  {/if}
  {#if rows.length}
    <ul class="divide-y divide-table-line rounded-lg border border-line-2">
      {#each rows as row (row.environment.revision_id)}
        <li>
          <button type="button" aria-pressed={selected?.revision_id === row.environment.revision_id}
            class="w-full break-words px-3 py-2 text-left hover:bg-muted-hover aria-pressed:bg-muted"
            onclick={() => inspect(row.environment)}>
            <span class="font-medium">{row.reason}</span>
            <span class="block text-xs text-muted-foreground-1">
              {row.environment.revision_id.slice(0, 8)} · {new Date(row.timestamp_unix_ms).toLocaleString()}
              {#if row.environment.revision_id === reference.revision_id} · Editor base{/if}
            </span>
          </button>
        </li>
      {/each}
    </ul>
  {:else if !loading && !pageError}
    <p class="text-muted-foreground-1">No committed revisions were found.</p>
  {/if}
  {#if cursor != null && !pageError}
    <button type="button" class="text-primary hover:underline disabled:opacity-50" disabled={loading}
      onclick={() => loadPage(cursor)}>Load older revisions</button>
  {/if}
  {#if reading}<p role="status" class="text-muted-foreground-1">Reconstructing revision…</p>{/if}
  {#if detailError}
    <p role="alert" class="text-destructive-text">{detailError}</p>
    <button type="button" class="text-primary hover:underline" onclick={() => selected && inspect(selected)}>Retry selected revision</button>
  {/if}
  {#if detail}
    <div class="space-y-2">
      <p class="break-all text-xs text-muted-foreground-1">Revision {detail.summary.environment.revision_id} · {detail.summary.actor}</p>
      {#if detail.summary.previous_revision_id}
        <p class="break-all text-xs text-muted-foreground-1">Previous revision {detail.summary.previous_revision_id}</p>
      {/if}
      {#if detail.summary.fork_parent}
        <p class="break-all text-xs text-muted-foreground-1">Forked from {detail.summary.fork_parent.environment_id}@{detail.summary.fork_parent.revision_id}</p>
      {/if}
      {#if detail.availability === RequestReviewAvailability.COMPLETE && detail.canonical_toml != null}
        <pre aria-label="Historical canonical TOML" class="max-h-96 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-line-2 bg-background-1 p-3 text-xs">{detail.canonical_toml}</pre>
        {#key detail.summary.environment.revision_id}
          <EnvironmentApplication revision={detail.summary.environment} />
        {/key}
        {#if detail.summary.fork_parent}
          {#key detail.summary.environment.revision_id}
            <EnvironmentPromotion fork={detail.summary.environment} forkToml={detail.canonical_toml}
              parentId={detail.summary.fork_parent.environment_id} />
          {/key}
        {/if}
      {:else}
        <p>{detail.availability === RequestReviewAvailability.PURGED ? 'Some historical values were purged.' : 'Historical content is unavailable.'}</p>
        {#if detail.purged_variables.length}<p class="break-words text-xs">Purged variables {detail.purged_variables.join(', ')}</p>{/if}
        {#if detail.partial_specification}
          <pre aria-label="Remaining historical settings" class="max-h-96 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-line-2 bg-background-1 p-3 text-xs">{JSON.stringify(detail.partial_specification, null, 2)}</pre>
        {/if}
      {/if}
    </div>
  {/if}
</section>
