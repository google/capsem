<script lang="ts">
  import { onMount } from 'svelte';
  import {
    RequestApplicationState, RequestDecision, RequestReviewAvailability, RequestState,
    type EnvironmentReviewContent, type RequestDetailResponse, type RequestSummary,
  } from '@capsem/sdk';
  import X from 'phosphor-svelte/lib/X';
  import { getEnvironmentRequest, resolveEnvironmentRequest } from '../../api';

  let { requests, names, onclose, onresolved }: {
    requests: RequestSummary[];
    names: Map<string, string | null | undefined>;
    onclose: () => void;
    onresolved: () => Promise<void>;
  } = $props();

  let dialog: HTMLDialogElement;
  let selected = $state<string | null>(null);
  let detail = $state<RequestDetailResponse | null>(null);
  let phase = $state<'loading' | 'resolving' | null>(null);
  let error = $state<string | null>(null);
  let reason = $state('');
  let readEpoch = 0;

  let reviewable = $derived(detail?.summary.decision === RequestState.PENDING
    && detail.proposed.availability === RequestReviewAvailability.COMPLETE
    && detail.base?.availability === RequestReviewAvailability.COMPLETE);
  type Specification = NonNullable<EnvironmentReviewContent['specification']>;
  const labels: Record<keyof Specification, string> = {
    name: 'Name', description: 'Description', image: 'Image', startup: 'Startup',
    variables: 'Environment variables', secrets: 'Secret bindings', repositories: 'Repositories',
    packages: 'Package access', skills: 'Skills', access: 'Access rules', detection: 'Detection rules',
    mcps: 'MCP servers and permissions', plugins: 'Plugins',
  };
  let changes = $derived.by(() => {
    const base = detail?.base?.specification;
    const proposed = detail?.proposed.specification;
    if (!base || !proposed) return [];
    return (Object.keys(labels) as (keyof Specification)[]).flatMap(field => {
      const before = display(base[field], field);
      const after = display(proposed[field], field);
      return before === after ? [] : [{ label: labels[field], before, after }];
    });
  });

  function display(value: Specification[keyof Specification], field: keyof Specification): string {
    if (value == null) return '—';
    if (field === 'startup' && typeof value === 'object' && 'script' in value && 'timeout_seconds' in value) {
      return `Timeout ${value.timeout_seconds} s\n\n${value.script}`;
    }
    return typeof value === 'string' ? value : JSON.stringify(value, null, 2);
  }

  function requester(request: RequestSummary): string {
    const id = request.requester?.session_id;
    return id ? names.get(id) ?? id : request.actor;
  }

  async function select(id: string) {
    const epoch = ++readEpoch;
    selected = id;
    detail = null;
    phase = 'loading';
    error = null;
    reason = '';
    try {
      const result = await getEnvironmentRequest(id);
      if (result.summary.request_id !== id) throw new Error('The gateway returned a different request');
      if (epoch === readEpoch) detail = result;
    } catch {
      if (epoch === readEpoch) error = 'The selected request could not be loaded.';
    } finally {
      if (epoch === readEpoch) phase = null;
    }
  }

  async function decide(decision: RequestDecision) {
    const current = detail;
    if (!current || phase || !reason.trim() || current.summary.decision !== RequestState.PENDING) return;
    if (decision === RequestDecision.APPROVE && !reviewable) return;
    const epoch = readEpoch;
    phase = 'resolving';
    error = null;
    try {
      const id = current.summary.request_id;
      const result = await resolveEnvironmentRequest(id, {
        decision, observed_revision: current.observed_revision, reason: reason.trim(),
      });
      if (result.request_id !== id) throw new Error('The gateway returned a different resolution');
      if (epoch === readEpoch) {
        detail = {
          ...current,
          summary: { ...current.summary, decision: result.decision, application: result.application, result: result.result },
        };
        error = result.application_error ?? null;
      }
      await onresolved();
    } catch {
      if (epoch === readEpoch) error = 'The decision could not be confirmed. Reload the request to check its outcome.';
    } finally {
      if (epoch === readEpoch) phase = null;
    }
  }

  onMount(() => {
    dialog.showModal();
    if (requests.length === 1) void select(requests[0].request_id);
    return () => { readEpoch++; };
  });
</script>

<dialog bind:this={dialog} {onclose} aria-labelledby="environment-approval-title"
  class="fixed inset-y-0 end-0 start-auto m-0 h-dvh max-h-none w-full max-w-2xl overflow-y-auto border-s border-overlay-border bg-overlay p-0 text-foreground shadow-xl backdrop:bg-overlay/70">
  <header class="flex items-center justify-between gap-3 border-b border-overlay-border p-4">
    <h2 id="environment-approval-title" class="text-lg font-medium">Environment approvals</h2>
    <button type="button" aria-label="Close environment approvals" class="rounded-lg p-2 hover:bg-muted-hover" onclick={() => dialog.close()}><X size={18} /></button>
  </header>

  <div class="space-y-4 p-5 text-sm">
    {#if !selected || requests.some(item => item.request_id !== selected)}
      <div class="flex flex-wrap gap-2">
        {#each requests as item (item.request_id)}
          <button type="button" aria-pressed={selected === item.request_id}
            disabled={phase === 'resolving'} class="rounded-lg border border-line-2 px-3 py-2 hover:bg-muted-hover"
            title={item.reason} onclick={() => select(item.request_id)}>{requester(item)}
            <span class="block max-w-xs truncate text-left text-xs text-muted-foreground-1">{item.reason}</span>
          </button>
        {:else}
          <p>No pending environment changes.</p>
        {/each}
      </div>
    {/if}
    {#if phase === 'loading'}<p role="status">Loading review…</p>{/if}
    {#if error}<p role="alert" class="text-destructive-text">{error}</p>{/if}
    {#if error && selected}
      <button type="button" class="text-primary hover:underline" onclick={() => selected && select(selected)}>Reload request</button>
    {/if}
    {#if detail}
      <div class="space-y-1">
        <h3 class="font-medium">{requester(detail.summary)}</h3>
        <p class="whitespace-pre-wrap break-words">{detail.summary.reason}</p>
        {#if detail.summary.requester}
          <p class="break-all text-xs text-muted-foreground-1">VM generation {detail.summary.requester.generation}</p>
        {/if}
        {#if detail.summary.base}
          <p class="break-all text-xs text-muted-foreground-1">Base revision {detail.summary.base.revision_id}</p>
        {/if}
        <p class="text-xs text-muted-foreground-1">Expires {new Date(detail.summary.expires_at_unix_ms).toLocaleString()}</p>
      </div>
      {#if detail.summary.decision === RequestState.PENDING}
        <p>Approval creates a fork for this VM. Access controls update live. Workload settings take effect on its next start.</p>
      {:else}
        <p role="status" class="capitalize">Decision {detail.summary.decision}
          {#if detail.summary.decision === RequestState.APPROVED} · Application {detail.summary.application}{/if}
        </p>
        {#if detail.summary.application === RequestApplicationState.PENDING && detail.summary.result}
          <p>Use the VM’s environment details to check whether a restart is pending.</p>
        {/if}
      {/if}
      {#if detail.base?.availability !== RequestReviewAvailability.COMPLETE || detail.proposed.availability !== RequestReviewAvailability.COMPLETE}
        <p class="text-muted-foreground-1">The complete review content is unavailable.</p>
      {:else}
        <section aria-label="Changed configuration" class="space-y-3">
          {#each changes as change (change.label)}
            <section class="space-y-2">
              <h4 class="font-medium">{change.label}</h4>
              <div class="grid gap-3 md:grid-cols-2">
                <div class="min-w-0 space-y-1">
                  <span class="text-xs text-muted-foreground-1">Base</span>
                  <pre class="max-h-[36vh] overflow-auto rounded-lg border border-card-line bg-card p-3 text-xs">{change.before}</pre>
                </div>
                <div class="min-w-0 space-y-1">
                  <span class="text-xs text-muted-foreground-1">Proposed</span>
                  <pre class="max-h-[36vh] overflow-auto rounded-lg border border-card-line bg-card p-3 text-xs">{change.after}</pre>
                </div>
              </div>
            </section>
          {:else}
            <p>No settings differ from the base revision.</p>
          {/each}
        </section>
      {/if}
      <details class="rounded-lg border border-card-line p-3">
        <summary class="cursor-pointer font-medium">Full revisions</summary>
        <div class="mt-3 grid gap-3 md:grid-cols-2">
          {#each [{ label: 'Base revision', value: detail.base }, { label: 'Proposed revision', value: detail.proposed }] as { label, value } (label)}
            <section class="min-w-0 space-y-2">
              <h4 class="font-medium">{label}</h4>
              {#if value?.specification}
                <pre class="max-h-[48vh] overflow-auto rounded-lg border border-card-line bg-card p-3 text-xs">{JSON.stringify(value.specification, null, 2)}</pre>
              {:else}
                <p>Review content is unavailable.</p>
              {/if}
            </section>
          {/each}
        </div>
      </details>
      {#if detail.summary.decision === RequestState.PENDING}
        <label class="block space-y-1">
          <span class="font-medium">Decision reason</span>
          <textarea bind:value={reason} rows={2} maxlength={4096} disabled={phase !== null}
            class="w-full rounded-lg border border-line-2 bg-layer px-3 py-2 focus:border-primary"></textarea>
        </label>
        <div class="flex flex-wrap gap-2">
          <button type="button" class="rounded-lg bg-primary px-3 py-2 text-primary-foreground disabled:opacity-50"
            disabled={!reviewable || phase !== null || !reason.trim()} onclick={() => decide(RequestDecision.APPROVE)}>Approve fork</button>
          <button type="button" class="rounded-lg border border-line-2 px-3 py-2 hover:bg-muted-hover disabled:opacity-50"
            disabled={phase !== null || !reason.trim()} onclick={() => decide(RequestDecision.DENY)}>Deny</button>
        </div>
      {/if}
    {/if}
  </div>
</dialog>
