<script lang="ts">
  import { onDestroy } from 'svelte';
  import { RequestApplicationState, RequestReviewAvailability, RequestState, type EnvironmentPromotionRequest,
    type EnvironmentRevisionReference } from '@capsem/sdk';
  import { getSavedEnvironment, getSavedEnvironmentRevision, promoteEnvironment } from '../../api';
  import { ApiError } from '../../gateway-sdk';
  import { sessionActionError } from '../../models/session-action-error';

  let { fork, forkToml, parentId }: {
    fork: EnvironmentRevisionReference;
    forkToml: string;
    parentId: string;
  } = $props();
  let reviewed = $state<{ revision: string; name: string; toml: string } | null>(null);
  let reason = $state('');
  let attempt = $state<EnvironmentPromotionRequest | null>(null);
  let receipt = $state<EnvironmentRevisionReference | null>(null);
  let error = $state('');
  let busy = $state(false);
  let active = true;
  const reader = new AbortController();
  const id = $props.id();

  onDestroy(() => { active = false; reader.abort(); });

  async function review() {
    busy = true;
    error = '';
    try {
      const parent = await getSavedEnvironment(parentId, reader.signal);
      if (parent.archived) throw new Error('The parent environment is archived.');
      const revision = parent.document.identity.revision_id;
      const detail = await getSavedEnvironmentRevision({ environment_id: parentId, revision_id: revision }, reader.signal);
      if (detail.availability !== RequestReviewAvailability.COMPLETE || detail.canonical_toml == null) {
        throw new Error('The exact parent revision is unavailable for review.');
      }
      if (active) reviewed = { revision, name: parent.document.specification.name, toml: detail.canonical_toml };
    } catch (cause: unknown) {
      if (active) error = sessionActionError(cause);
    } finally {
      if (active) busy = false;
    }
  }

  async function submit() {
    if (!reviewed || (attempt === null && !reason.trim())) return;
    const request = attempt ?? {
      request_id: crypto.randomUUID(), base_revision_id: reviewed.revision, source: fork, reason: reason.trim(),
    };
    attempt = request;
    busy = true;
    error = '';
    try {
      const result = await promoteEnvironment(parentId, request);
      if (!active) return;
      if (result.decision === RequestState.APPROVED && result.application === RequestApplicationState.APPLIED
          && result.result?.environment_id === parentId) {
        receipt = result.result;
      } else {
        error = `Promotion request ${result.request_id} is ${result.decision}, with application ${result.application}.`;
      }
    } catch (cause: unknown) {
      if (!active) return;
      if (cause instanceof ApiError && (cause.status === 409 || cause.status === 412)) {
        reviewed = null;
        attempt = null;
        error = 'The reviewed revisions or their admission changed. Load the current parent and review again.';
      } else {
        error = sessionActionError(cause);
      }
    } finally {
      if (active) busy = false;
    }
  }
</script>

<section aria-label="Fork promotion" class="space-y-3 border-t border-line-2 pt-3">
  {#if receipt}
    <p role="status" class="break-all">Promoted to parent revision {receipt.revision_id}.</p>
  {:else if reviewed}
    <p>Review the exact revision that will replace the saved configuration for future sandboxes in {reviewed.name}.
      Running sandboxes keep their pinned revisions.</p>
    <div class="grid gap-3 md:grid-cols-2">
      <div>
        <p class="break-all pb-1 text-xs text-muted-foreground-1">Parent revision {reviewed.revision}</p>
        <pre aria-label="Current parent configuration" class="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-line-2 p-2 text-xs">{reviewed.toml}</pre>
      </div>
      <div>
        <p class="break-all pb-1 text-xs text-muted-foreground-1">Fork revision {fork.revision_id}</p>
        <pre aria-label="Reviewed fork configuration" class="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-line-2 p-2 text-xs">{forkToml}</pre>
      </div>
    </div>
    <label for={id} class="block font-medium">Reason for promotion</label>
    <input {id} bind:value={reason} disabled={busy || attempt !== null}
      class="w-full rounded-lg border border-line-2 bg-background-1 px-3 py-2" />
    {#if attempt && error}<p class="break-all text-xs text-muted-foreground-1">Request {attempt.request_id}</p>{/if}
    <button type="button" disabled={busy || !reason.trim()} onclick={submit}
      class="rounded-lg bg-primary px-3 py-2 font-medium text-primary-foreground hover:bg-primary-hover disabled:opacity-50">
      {busy ? 'Promoting…' : attempt ? 'Check promotion' : 'Promote to parent'}
    </button>
    {#if !attempt}
      <button type="button" disabled={busy} onclick={() => { reviewed = null; error = ''; }}
        class="px-3 py-2 text-primary hover:underline">Cancel review</button>
    {/if}
  {:else}
    <button type="button" disabled={busy} onclick={review}
      class="rounded-lg border border-line-2 px-3 py-2 font-medium hover:bg-muted-hover disabled:opacity-50">
      {busy ? 'Loading parent…' : 'Review promotion to parent'}
    </button>
  {/if}
  {#if error}<p role="alert" class="text-destructive-text">{error}</p>{/if}
</section>
