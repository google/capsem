<script lang="ts">
  import { onMount } from 'svelte';
  import type { EnvironmentListItem, EnvironmentRevisionReference } from '@capsem/sdk';
  import { listSavedEnvironments } from '../../api';
  import { sessionActionError } from '../../models/session-action-error';

  let { onselect, onedit, disabled = false, refreshVersion = 0 }: {
    onselect: (item: EnvironmentListItem) => void;
    onedit: (reference: EnvironmentRevisionReference | null) => void;
    disabled?: boolean;
    refreshVersion?: number;
  } = $props();
  let filter = $state('');
  let rows = $state<EnvironmentListItem[]>([]);
  let cursor = $state<string | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let refresh = $state(0);
  let controller: AbortController | null = null;
  let currentSearch = '';
  let searchInput: HTMLInputElement;
  let searchId = $props.id();
  onMount(() => searchInput.focus());

  async function load(search: string, after: string | null, signal: AbortSignal) {
    loading = true;
    error = null;
    try {
      const page = await listSavedEnvironments({
        ...(search ? { search } : {}), ...(after ? { after } : {}), signal,
      });
      if (signal.aborted) return;
      rows = after ? [...rows, ...page.environments] : page.environments;
      cursor = page.next_cursor ?? null;
    } catch (err: unknown) {
      if (!signal.aborted) error = sessionActionError(err);
    } finally {
      if (!signal.aborted) loading = false;
    }
  }

  $effect(() => {
    const search = filter.trim();
    const delayMs = refresh + refreshVersion > 0 && search === currentSearch ? 0 : search ? 200 : 0;
    currentSearch = search;
    const current = new AbortController();
    controller = current;
    rows = [];
    cursor = null;
    loading = true;
    error = null;
    const delay = window.setTimeout(() => { void load(search, null, current.signal); }, delayMs);
    return () => { current.abort(); window.clearTimeout(delay); };
  });

  function onkey(event: KeyboardEvent) {
    if (event.key === 'Enter') event.stopPropagation();
  }
</script>

<div class="space-y-2" aria-busy={loading}>
  <label for={searchId} class="text-sm font-medium text-foreground">Search saved environments</label>
  <div class="flex items-center gap-2">
    <input id={searchId} bind:this={searchInput} bind:value={filter} type="search" maxlength="64" onkeydown={onkey} {disabled}
      class="min-w-0 flex-1 px-3 py-2 rounded-lg bg-background-1 border border-line-2 focus:border-primary outline-hidden text-sm text-foreground" />
    <button type="button" class="text-xs text-primary hover:underline disabled:opacity-50" onclick={() => refresh++}
      onkeydown={onkey} disabled={loading || disabled}>{error ? 'Retry' : 'Refresh'}</button>
  </div>
  <button type="button" class="text-xs text-primary hover:underline disabled:opacity-50" {disabled}
    onkeydown={onkey} onclick={() => onedit(null)}>New environment</button>
  {#if error}<p role="alert" class="text-sm text-destructive break-words">{error}</p>{/if}
  {#if loading}<p role="status" class="text-xs text-muted-foreground-1">Loading saved environments…</p>{/if}
  {#if !loading && !error && rows.length === 0 && !cursor}
    <p class="text-sm text-muted-foreground-1">{filter.trim() ? 'No matching environments.' : 'No saved environments.'}</p>
  {/if}
  {#if rows.length}
    <ul class="max-h-60 overflow-y-auto divide-y divide-table-line rounded-lg border border-line-2">
      {#each rows as row (row.environment.environment_id)}
        <li class="p-3 text-sm">
          {#if row.summary}
            <div class="flex items-start justify-between gap-3">
              <div class="min-w-0">
                <p class="font-medium text-foreground break-words">{row.summary.name}</p>
                {#if row.summary.description}
                  <p class="text-xs text-muted-foreground-1 line-clamp-2" title={row.summary.description}>{row.summary.description}</p>
                {/if}
              </div>
              <div class="flex shrink-0 gap-3">
                <button type="button" class="text-xs text-primary hover:underline focus-visible:outline-primary disabled:opacity-50"
                  aria-label={`Edit ${row.summary.name}`} onkeydown={onkey} onclick={() => onedit(row.environment)} {disabled}>Edit</button>
                <button type="button" class="text-xs text-primary hover:underline focus-visible:outline-primary disabled:opacity-50"
                  aria-label={`Use ${row.summary.name}`} onkeydown={onkey} onclick={() => onselect(row)} {disabled}>Use</button>
              </div>
            </div>
            <p class="mt-1 text-xs text-muted-foreground-1 break-all">
              {#if row.summary.image}
                <span title={row.summary.image.resolved_digest}>{row.summary.image.reference} · {row.summary.image.update_policy} updates</span>
              {:else}Plain VM{/if}
              {#if row.instance_count != null} · {row.instance_count} {row.instance_count === 1 ? 'session' : 'sessions'}{/if}
            </p>
            <p class="text-xs text-muted-foreground-1" title={row.environment.revision_id}>
              Revision {row.environment.revision_id.slice(0, 8)} · {new Date(row.summary.timestamp_unix_ms).toLocaleString()}
            </p>
          {:else}
            <p class="text-muted-foreground-1">Unavailable environment</p>
            <p class="text-xs text-muted-foreground-1 break-all">{row.environment.environment_id}</p>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if cursor}
    <button type="button" class="text-sm text-primary hover:underline disabled:opacity-50" disabled={loading || disabled}
      onkeydown={onkey} onclick={() => { if (controller) void load(filter.trim(), cursor, controller.signal); }}>Load more</button>
  {/if}
</div>
