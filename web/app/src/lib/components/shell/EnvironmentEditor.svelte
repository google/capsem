<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { EnvironmentArchiveAction, RequestApplicationState, RequestReviewAvailability, RequestState, type EnvironmentArchiveRequest, type EnvironmentCreateRequest, type EnvironmentCurrentResponse,
    type EnvironmentMutationResponse, type EnvironmentRevisionReference } from '@capsem/sdk';
  import X from 'phosphor-svelte/lib/X';
  import { archiveEnvironment, getSavedEnvironment, getSavedEnvironmentRevision, saveEnvironment, validateSavedEnvironmentToml } from '../../api';
  import { ApiError } from '../../gateway-sdk';
  import { editableFields, environmentChanges, reviewedDraft, type EnvironmentDraft } from '../../models/environment-editor';
  import { sessionActionError } from '../../models/session-action-error';
  import EnvironmentFields from './EnvironmentFields.svelte';
  import EnvironmentHistory from './EnvironmentHistory.svelte';

  let { selection, onclose, onsaved }: {
    selection: EnvironmentRevisionReference | null;
    onclose: () => void;
    onsaved: () => void;
  } = $props();
  type Phase = 'loading' | 'loadFailed' | 'editing' | 'validating' | 'reviewing' | 'saving' | 'stale' | 'unconfirmed' | 'saved';
  let phase = $state<Phase>('editing');
  let current = $state<EnvironmentCurrentResponse | null>(null);
  let fields = $state(editableFields());
  let draft = $state<EnvironmentDraft | null>(null);
  let attempt = $state<EnvironmentCreateRequest | EnvironmentArchiveRequest | null>(null);
  let archiveReview = $state(false);
  let saved = $state<EnvironmentRevisionReference | null>(null);
  let reason = $state('');
  let error = $state<string | null>(null);
  let source = $state<'fields' | 'toml'>('fields');
  let showHistory = $state(false);
  let canonical = $state<string | null>(null);
  let toml = $state('');
  let loadingToml = $state(false);
  let dialog: HTMLDialogElement;
  let title: HTMLHeadingElement;
  let content: HTMLDivElement;
  let reader: AbortController | null = null;
  let tomlReader: AbortController | null = null;
  const id = $props.id();
  let base = $derived(current ? { environment_id: current.document.identity.id, revision_id: current.document.identity.revision_id } : null);
  let original = $derived(current ? reviewedDraft(editableFields(current.document.specification)) : null);
  let changes = $derived(draft ? environmentChanges(original, draft) : []);
  let dirtyToml = $derived(canonical != null && toml !== canonical);
  const action = 'rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary-hover disabled:opacity-50';
  const secondary = 'rounded-lg border border-line-2 px-4 py-2 text-sm font-medium hover:bg-muted-hover aria-pressed:bg-muted aria-pressed:border-primary';

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
      archiveReview = false;
      reason = '';
      source = 'fields';
      showHistory = value.archived;
      canonical = null;
      toml = '';
      phase = 'editing';
    } catch (err: unknown) {
      if (!controller.signal.aborted) {
        error = sessionActionError(err);
        phase = 'loadFailed';
      }
    }
  }

  async function loadToml() {
    if (!base) return;
    tomlReader?.abort();
    const controller = new AbortController();
    tomlReader = controller;
    loadingToml = true;
    error = null;
    try {
      const value = await getSavedEnvironmentRevision(base, controller.signal);
      if (controller.signal.aborted) return;
      if (value.availability !== RequestReviewAvailability.COMPLETE || value.canonical_toml == null) {
        error = 'Canonical TOML is unavailable for this revision.';
        return;
      }
      canonical = value.canonical_toml;
      toml = value.canonical_toml;
    } catch (err) {
      if (!controller.signal.aborted) error = sessionActionError(err);
    } finally {
      if (!controller.signal.aborted) loadingToml = false;
    }
  }

  function openToml() {
    if (source === 'fields' && !current?.archived) {
      try {
        if (environmentChanges(original, reviewedDraft(fields)).length) throw new Error();
      } catch {
        error = 'Review or discard the current settings draft before switching to TOML.';
        return;
      }
    }
    source = 'toml';
    showHistory = false;
    error = null;
    if (canonical == null && !loadingToml) void loadToml();
  }

  function openSettings() {
    if (dirtyToml) {
      error = 'Review or discard the TOML draft before switching to settings.';
      return;
    }
    source = 'fields';
    showHistory = false;
    error = null;
  }

  async function review() {
    if (phase !== 'editing' || showHistory || current?.archived) return;
    error = null;
    draft = null;
    try {
      let next: EnvironmentDraft;
      if (source === 'toml') {
        if (!base || canonical == null) return;
        const controller = new AbortController();
        tomlReader = controller;
        phase = 'validating';
        next = (await validateSavedEnvironmentToml(base, toml, controller.signal)).specification;
        if (controller.signal.aborted) return;
        phase = 'editing';
      } else {
        next = reviewedDraft(fields);
      }
      if (!environmentChanges(original, next).length) { error = 'There are no changes to save.'; return; }
      draft = next;
      attempt = null;
      archiveReview = false;
      phase = 'reviewing';
      focusTitle();
    } catch (err) {
      if (tomlReader?.signal.aborted) return;
      if (source === 'toml' && err instanceof ApiError && err.status === 409) {
        phase = 'stale';
        error = 'The saved revision changed before TOML could be validated. Reload the current revision to start a new edit.';
      } else {
        phase = 'editing';
        error = sessionActionError(err);
      }
    }
  }

  function reviewArchive() {
    if (phase !== 'editing' || !base || current?.archived) return;
    archiveReview = true;
    attempt = null;
    reason = '';
    error = null;
    phase = 'reviewing';
    focusTitle();
  }

  async function save() {
    if (phase !== 'reviewing' && phase !== 'unconfirmed') return;
    if (!attempt) {
      if (!reason.trim()) return;
      const identity = { request_id: crypto.randomUUID(), reason: reason.trim() };
      if (archiveReview && base) {
        attempt = { ...identity, action: EnvironmentArchiveAction.ARCHIVE, base_revision_id: base.revision_id };
      } else if (draft) {
        attempt = { ...identity, specification: draft };
      } else return;
    }
    if ('action' in attempt && !base) return;
    phase = 'saving';
    error = null;
    try {
      let receipt: EnvironmentMutationResponse;
      if ('action' in attempt) {
        receipt = await archiveEnvironment(base!.environment_id, attempt);
      } else {
        receipt = await saveEnvironment(attempt, base);
      }
      if (receipt.decision !== RequestState.APPROVED || receipt.application !== RequestApplicationState.APPLIED
          || !receipt.result || receipt.result.environment_id !== receipt.environment_id
          || (archiveReview && receipt.result.revision_id !== base?.revision_id)) {
        phase = 'unconfirmed';
        error = archiveReview ? 'The gateway has yet to confirm archival.' : 'The gateway has yet to confirm the saved revision.';
        return;
      }
      saved = receipt.result;
      phase = 'saved';
      onsaved();
      focusTitle();
    } catch (err) {
      if (err instanceof ApiError && err.status === 412) {
        phase = 'editing';
        error = 'A catalog selection changed. Browse the catalog again and review the updated draft.';
        attempt = null;
        draft = null;
        focusTitle();
      } else if (err instanceof ApiError && err.status === 409) {
        phase = 'stale';
        error = 'The saved environment changed. Reload the current revision.';
      } else if (err instanceof ApiError && [400, 403, 404, 413, 422].includes(err.status)) {
        phase = 'reviewing';
        error = sessionActionError(err);
        attempt = null;
      } else {
        phase = 'unconfirmed';
        error = 'The gateway could not confirm the change. Retry to check the same request.';
      }
    }
  }

  function close() { if (phase !== 'saving') dialog.close(); }

  onMount(() => {
    dialog.showModal();
    focusTitle();
    if (selection) void load(selection.revision_id);
    return () => { reader?.abort(); tomlReader?.abort(); };
  });
</script>

<dialog bind:this={dialog} {onclose} aria-labelledby={`${id}-title`}
  onkeydown={(event) => event.stopPropagation()}
  oncancel={(event) => { if (phase === 'saving') event.preventDefault(); }}
  class="fixed inset-y-0 end-0 start-auto m-0 h-dvh max-h-none w-full max-w-2xl overflow-hidden border-s border-overlay-border bg-overlay p-0 text-foreground shadow-xl backdrop:bg-overlay/70">
  <div class="flex h-full flex-col">
  <header class="flex shrink-0 items-center justify-between gap-3 border-b border-overlay-border p-4">
    <h2 id={`${id}-title`} bind:this={title} tabindex="-1" class="text-lg font-medium">
      {phase === 'reviewing' ? (archiveReview ? 'Review environment archive' : 'Review environment changes')
        : current?.archived ? 'Archived environment' : selection ? 'Edit saved environment' : 'New saved environment'}
    </h2>
    <button type="button" aria-label="Close environment editor" class="rounded-lg p-2 hover:bg-muted-hover disabled:opacity-50" disabled={phase === 'saving'} onclick={close}><X size={18} /></button>
  </header>

  <div bind:this={content} class="min-h-0 flex-1 space-y-4 overflow-y-auto p-5 text-sm">
    {#if error}<p role="alert" class="rounded-lg border border-destructive/30 p-3 text-destructive-text">{error}</p>{/if}
    {#if phase === 'loading'}<p role="status">Loading saved revision…</p>{/if}
    {#if base && phase !== 'saved'}<p class="break-all text-xs text-muted-foreground-1">Base revision {base.revision_id}</p>{/if}
    {#if base && (phase === 'editing' || phase === 'validating')}
      <nav aria-label="Environment editor views" class="flex flex-wrap gap-2">
        {#if !current?.archived}<button type="button" class={secondary} aria-pressed={!showHistory && source === 'fields'} disabled={phase === 'validating'} onclick={openSettings}>Settings</button>{/if}
        <button type="button" class={secondary} aria-pressed={!showHistory && source === 'toml'} disabled={phase === 'validating'} onclick={openToml}>TOML</button>
        <button type="button" class={secondary} aria-pressed={showHistory} disabled={phase === 'validating'} onclick={() => { error = null; showHistory = true; }}>Change history</button>
      </nav>
    {/if}
    {#if showHistory && base && phase === 'editing'}
      <EnvironmentHistory reference={base} />
    {:else if source === 'toml' && (phase === 'editing' || phase === 'validating' || (phase === 'stale' && !draft))}
      {#if loadingToml}<p role="status">Loading canonical TOML…</p>{/if}
      {#if canonical != null}
        <div class="space-y-2">
          <label for={`${id}-toml`} class="font-medium">Environment TOML</label>
          <textarea id={`${id}-toml`} class="w-full rounded-lg border border-line-2 bg-background-1 p-3 font-mono text-xs"
            rows="16" bind:value={toml} readonly={phase !== 'editing' || current?.archived} spellcheck="false"></textarea>
          {#if dirtyToml && phase === 'editing'}
            <button type="button" class="text-primary hover:underline" onclick={() => { toml = canonical ?? ''; error = null; }}>Discard TOML changes</button>
          {/if}
        </div>
      {:else if !loadingToml}
        <button type="button" class="text-primary hover:underline" onclick={loadToml}>Retry canonical TOML</button>
      {/if}
    {:else if phase === 'editing'}
      <EnvironmentFields bind:fields reference={base} pin={current?.document.specification.image?.resolved_digest} />
    {:else if archiveReview && phase !== 'saved'}
      <p>New sandboxes cannot use this environment after it is archived. Existing sandboxes keep their saved revisions.</p>
      <div class="space-y-1">
        <label for={`${id}-archive-reason`} class="text-sm font-medium">Reason for archiving</label>
        <textarea id={`${id}-archive-reason`} class="w-full rounded-lg border border-line-2 bg-background-1 p-2 text-sm" rows="2" bind:value={reason} disabled={phase !== 'reviewing'}></textarea>
      </div>
      {#if attempt && phase === 'unconfirmed'}<p class="break-all text-xs text-muted-foreground-1">Request {attempt.request_id}</p>{/if}
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
    {#if phase === 'saving'}<p role="status">{archiveReview ? 'Archiving environment…' : 'Saving environment…'}</p>{/if}
    {#if saved}
      <p role="status" class="break-all">{archiveReview ? 'Archived environment' : `Saved revision ${saved.revision_id}`}</p>
    {/if}
  </div>

  <footer class="flex shrink-0 flex-wrap justify-end gap-2 border-t border-overlay-border p-4">
    {#if phase === 'editing'}
      <button type="button" class={secondary} onclick={close}>{current?.archived ? 'Close' : 'Cancel'}</button>
      {#if base && !current?.archived}<button type="button" class={secondary} onclick={reviewArchive}>Archive environment</button>{/if}
      {#if !showHistory && !current?.archived}<button type="button" class={action} disabled={source === 'toml' && canonical == null} onclick={review}>Review changes</button>{/if}
    {:else if phase === 'validating'}
      <p role="status">Validating TOML…</p>
    {:else if phase === 'reviewing'}
      <button type="button" class={secondary} onclick={() => { error = null; phase = 'editing'; focusTitle(); }}>Back to editing</button>
      <button type="button" class={action} disabled={!reason.trim()} onclick={save}>{archiveReview ? 'Confirm archive' : 'Save environment'}</button>
    {:else if phase === 'stale' && selection}
      <button type="button" class={secondary} onclick={close}>Close</button>
      <button type="button" class={action} onclick={() => load()}>{draft || dirtyToml ? 'Discard draft and reload' : 'Load current revision'}</button>
    {:else if phase === 'loadFailed'}
      <button type="button" class={action} onclick={() => load(selection?.revision_id)}>Retry</button>
    {:else if phase === 'unconfirmed'}
      <button type="button" class={secondary} onclick={close}>Close</button>
      <button type="button" class={action} onclick={save}>{archiveReview ? 'Retry same archive' : 'Retry same save'}</button>
    {:else if phase === 'saved'}
      <button type="button" class={action} onclick={close}>Done</button>
    {/if}
  </footer>
  </div>
</dialog>
