<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { RequestApplicationState, RequestState, type EnvironmentCreateRequest, type EnvironmentCurrentResponse,
    type EnvironmentRevisionReference } from '@capsem/sdk';
  import X from 'phosphor-svelte/lib/X';
  import { getSavedEnvironment, saveEnvironment } from '../../api';
  import { ApiError } from '../../gateway-sdk';
  import { editableFields, environmentChanges, reviewedDraft, type EnvironmentDraft } from '../../models/environment-editor';
  import { sessionActionError } from '../../models/session-action-error';
  import EnvironmentFields from './EnvironmentFields.svelte';

  let { selection, onclose, onsaved }: {
    selection: EnvironmentRevisionReference | null;
    onclose: () => void;
    onsaved: () => void;
  } = $props();
  type Phase = 'loading' | 'loadFailed' | 'editing' | 'reviewing' | 'saving' | 'stale' | 'unconfirmed' | 'saved';
  let phase = $state<Phase>('editing');
  let current = $state<EnvironmentCurrentResponse | null>(null);
  let fields = $state(editableFields());
  let draft = $state<EnvironmentDraft | null>(null);
  let attempt = $state<EnvironmentCreateRequest | null>(null);
  let saved = $state<EnvironmentRevisionReference | null>(null);
  let reason = $state('');
  let error = $state<string | null>(null);
  let dialog: HTMLDialogElement;
  let title: HTMLHeadingElement;
  let content: HTMLDivElement;
  let reader: AbortController | null = null;
  const id = $props.id();
  let base = $derived(current ? { environment_id: current.document.identity.id, revision_id: current.document.identity.revision_id } : null);
  let changes = $derived(draft ? environmentChanges(current ? reviewedDraft(editableFields(current.document.specification)) : null, draft) : []);
  const action = 'rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary-hover disabled:opacity-50';
  const secondary = 'rounded-lg border border-line-2 px-4 py-2 text-sm font-medium hover:bg-muted-hover';

  function focusTitle() {
    void tick().then(() => {
      if (!dialog?.open) return;
      content.scrollTop = 0;
      title.focus();
    });
  }

  async function load(expectedRevision: string | undefined = undefined) {
    if (!selection) return;
    reader?.abort();
    const controller = new AbortController();
    reader = controller;
    phase = 'loading';
    error = null;
    try {
      const value = await getSavedEnvironment(selection.environment_id, controller.signal);
      if (controller.signal.aborted) return;
      if (expectedRevision && value.document.identity.revision_id !== expectedRevision) {
        error = 'The saved revision changed. Load the current revision to edit it.';
        phase = 'stale';
        return;
      }
      current = value;
      fields = editableFields(value.document.specification);
      draft = null;
      attempt = null;
      reason = '';
      phase = 'editing';
    } catch (err: unknown) {
      if (!controller.signal.aborted) {
        error = sessionActionError(err);
        phase = 'loadFailed';
      }
    }
  }

  function review() {
    error = null;
    try {
      const next = reviewedDraft(fields);
      const before = current ? reviewedDraft(editableFields(current.document.specification)) : null;
      if (!environmentChanges(before, next).length) { error = 'There are no changes to save.'; return; }
      draft = next;
      attempt = null;
      phase = 'reviewing';
      focusTitle();
    } catch (err) {
      error = sessionActionError(err);
    }
  }

  async function save() {
    if (phase !== 'reviewing' && phase !== 'unconfirmed') return;
    if (!attempt) {
      if (!draft || !reason.trim()) return;
      attempt = { request_id: crypto.randomUUID(), reason: reason.trim(), specification: draft };
    }
    phase = 'saving';
    error = null;
    try {
      const receipt = await saveEnvironment(attempt, base);
      if (receipt.decision !== RequestState.APPROVED || receipt.application !== RequestApplicationState.APPLIED
          || !receipt.result || receipt.result.environment_id !== receipt.environment_id) {
        phase = 'unconfirmed';
        error = 'The gateway has yet to confirm the saved revision.';
        return;
      }
      saved = receipt.result;
      phase = 'saved';
      onsaved();
      focusTitle();
    } catch (err) {
      if (err instanceof ApiError && err.status === 409) {
        phase = 'stale';
        error = 'This save conflicted with the saved environment. Reload the current revision to start a new edit.';
      } else if (err instanceof ApiError && [400, 403, 404, 413, 422].includes(err.status)) {
        phase = 'reviewing';
        error = sessionActionError(err);
        attempt = null;
      } else {
        phase = 'unconfirmed';
        error = 'The save could not be confirmed. Retry this save to check the same request.';
      }
    }
  }

  function close() { if (phase !== 'saving') dialog.close(); }

  onMount(() => {
    dialog.showModal();
    focusTitle();
    if (selection) void load(selection.revision_id);
    return () => reader?.abort();
  });
</script>

<dialog bind:this={dialog} {onclose} aria-labelledby={`${id}-title`}
  onkeydown={(event) => event.stopPropagation()}
  oncancel={(event) => { if (phase === 'saving') event.preventDefault(); }}
  class="fixed inset-y-0 end-0 start-auto m-0 h-dvh max-h-none w-full max-w-2xl overflow-hidden border-s border-overlay-border bg-overlay p-0 text-foreground shadow-xl backdrop:bg-overlay/70">
  <div class="flex h-full flex-col">
  <header class="flex shrink-0 items-center justify-between gap-3 border-b border-overlay-border p-4">
    <h2 id={`${id}-title`} bind:this={title} tabindex="-1" class="text-lg font-medium">
      {phase === 'reviewing' ? 'Review environment changes' : selection ? 'Edit saved environment' : 'New saved environment'}
    </h2>
    <button type="button" aria-label="Close environment editor" class="rounded-lg p-2 hover:bg-muted-hover disabled:opacity-50" disabled={phase === 'saving'} onclick={close}><X size={18} /></button>
  </header>

  <div bind:this={content} class="min-h-0 flex-1 space-y-4 overflow-y-auto p-5 text-sm">
    {#if error}<p role="alert" class="rounded-lg border border-destructive/30 p-3 text-destructive-text">{error}</p>{/if}
    {#if phase === 'loading'}<p role="status">Loading saved revision…</p>{/if}
    {#if base && phase !== 'saved'}<p class="break-all text-xs text-muted-foreground-1">Base revision {base.revision_id}</p>{/if}
    {#if phase === 'editing'}
      <EnvironmentFields bind:fields pin={current?.document.specification.image?.resolved_digest} />
    {:else if draft && phase !== 'saved'}
      <div class="space-y-3" aria-label="Changed settings" role="region">
        {#each changes as change (change.label)}
          <div class="space-y-1">
            <h3 class="text-sm font-medium">{change.label}</h3>
            <div class="grid gap-2 sm:grid-cols-2">
              <div><p class="text-xs text-muted-foreground-1">Saved</p><pre class="whitespace-pre-wrap break-words rounded-md border border-line-2 p-2 text-xs">{change.before}</pre></div>
              <div><p class="text-xs text-muted-foreground-1">Proposed</p><pre class="whitespace-pre-wrap break-words rounded-md border border-line-2 p-2 text-xs">{change.after}</pre></div>
            </div>
          </div>
        {/each}
      </div>
      <div class="space-y-1">
        <label for={`${id}-reason`} class="text-sm font-medium">Reason for saving</label>
        <textarea id={`${id}-reason`} class="w-full rounded-lg border border-line-2 bg-background-1 p-2 text-sm" rows="2" bind:value={reason} disabled={phase !== 'reviewing'}></textarea>
      </div>
      {#if attempt && phase === 'unconfirmed'}<p class="break-all text-xs text-muted-foreground-1">Request {attempt.request_id}</p>{/if}
    {/if}
    {#if phase === 'saving'}<p role="status">Saving environment…</p>{/if}
    {#if saved}
      <p role="status" class="break-all">Saved revision {saved.revision_id}</p>
    {/if}
  </div>

  <footer class="flex shrink-0 flex-wrap justify-end gap-2 border-t border-overlay-border p-4">
    {#if phase === 'editing'}
      <button type="button" class={secondary} onclick={close}>Cancel</button>
      <button type="button" class={action} onclick={review}>Review changes</button>
    {:else if phase === 'reviewing'}
      <button type="button" class={secondary} onclick={() => { error = null; phase = 'editing'; focusTitle(); }}>Back to editing</button>
      <button type="button" class={action} disabled={!reason.trim()} onclick={save}>Save environment</button>
    {:else if phase === 'stale' && selection}
      <button type="button" class={secondary} onclick={close}>Close</button>
      <button type="button" class={action} onclick={() => load()}>{draft ? 'Discard draft and reload' : 'Load current revision'}</button>
    {:else if phase === 'loadFailed'}
      <button type="button" class={action} onclick={() => load(selection?.revision_id)}>Retry</button>
    {:else if phase === 'unconfirmed'}
      <button type="button" class={secondary} onclick={close}>Close</button>
      <button type="button" class={action} onclick={save}>Retry same save</button>
    {:else if phase === 'saved'}
      <button type="button" class={action} onclick={close}>Done</button>
    {/if}
  </footer>
  </div>
</dialog>
