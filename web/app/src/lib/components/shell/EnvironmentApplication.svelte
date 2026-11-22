<script lang="ts">
  import { onMount } from 'svelte';
  import { RequestApplicationState, VmLifecycleState, type EnvironmentApplyRequest, type EnvironmentRevisionReference,
    type RequestDetailResponse } from '@capsem/sdk';
  import { applySavedEnvironment, getEnvironmentRequest, getSavedEnvironment, getVmInfo,
    onEnvironmentRequestsChanged } from '../../api';
  import { ApiError } from '../../gateway-sdk';
  import { sessionActionError } from '../../models/session-action-error';
  import { vmStore } from '../../stores/vms.svelte.ts';

  let { revision }: { revision: EnvironmentRevisionReference } = $props();
  let phase = $state<'closed' | 'select' | 'review' | 'receipt'>('closed');
  let selected = $state<string[]>([]);
  let reviewed = $state<Pick<EnvironmentApplyRequest, 'base_revision_id' | 'targets'> | null>(null);
  let attempt = $state<EnvironmentApplyRequest | null>(null);
  let detail = $state<RequestDetailResponse | null>(null);
  let reason = $state('');
  let error = $state('');
  let busy = $state(false);
  let active = true;
  let epoch = 0;
  const reader = new AbortController();
  const id = $props.id();
  let running = $derived(vmStore.vms.filter(vm => vm.status === VmLifecycleState.RUNNING));
  let retryable = $derived((detail?.applications ?? [])
    .filter(item => item.state === RequestApplicationState.FAILED && running.some(vm => vm.id === item.target.session_id))
    .map(item => item.target.session_id));

  onMount(() => {
    const stop = onEnvironmentRequestsChanged(id => {
      if (attempt?.request_id === id && phase === 'receipt') void readReceipt();
    }, () => { if (phase === 'receipt') void readReceipt(); });
    return () => { active = false; reader.abort(); stop(); };
  });

  async function review() {
    if (!selected.length) return;
    busy = true;
    error = '';
    try {
      const environment = await getSavedEnvironment(revision.environment_id, reader.signal);
      if (environment.archived) throw new Error('The saved environment is archived.');
      const targets = await Promise.all(selected.map(async id => {
        const vm = await getVmInfo(id);
        if (!vm.environment?.desired || !vm.environment.owner_generation || vm.status !== VmLifecycleState.RUNNING) {
          throw new Error(`Sandbox ${vm.name ?? id} no longer has a running environment generation.`);
        }
        return { session_id: id, generation: vm.environment.owner_generation, base: vm.environment.desired };
      }));
      if (active) {
        reviewed = { base_revision_id: environment.document.identity.revision_id, targets };
        phase = 'review';
      }
    } catch (cause: unknown) {
      if (active) error = sessionActionError(cause);
    } finally {
      if (active) busy = false;
    }
  }

  async function readReceipt() {
    if (!attempt) return;
    const read = ++epoch;
    try {
      const result = await getEnvironmentRequest(attempt.request_id);
      if (active && read === epoch) { detail = result; error = ''; }
    } catch (cause: unknown) {
      if (active && read === epoch) error = sessionActionError(cause);
    }
  }

  async function submit() {
    if (!reviewed || !reason.trim()) return;
    attempt ??= { ...reviewed, request_id: crypto.randomUUID(), revision_id: revision.revision_id, reason: reason.trim() };
    busy = true;
    error = '';
    try {
      await applySavedEnvironment(revision.environment_id, attempt);
      if (active) {
        phase = 'receipt';
        await readReceipt();
      }
    } catch (cause: unknown) {
      if (!active) return;
      if (cause instanceof ApiError && (cause.status === 409 || cause.status === 412)) {
        attempt = null;
        reviewed = null;
        phase = 'select';
        error = 'The saved revision or a selected sandbox changed. Review the targets again.';
      } else error = sessionActionError(cause);
    } finally {
      if (active) busy = false;
    }
  }

  function reviewFailedTargets() {
    epoch++;
    selected = [...retryable];
    reviewed = null;
    attempt = null;
    detail = null;
    reason = '';
    error = '';
    phase = 'select';
  }
</script>

<section aria-label="Apply saved revision" class="space-y-3 border-t border-line-2 pt-3">
  {#if phase === 'closed'}
    <button type="button" class="rounded-lg border border-line-2 px-3 py-2 font-medium hover:bg-muted-hover"
      onclick={() => phase = 'select'}>Apply to running sandboxes</button>
  {:else if phase === 'select'}
    <fieldset disabled={busy} class="space-y-2">
      <legend class="font-medium">Select running sandboxes</legend>
      {#each running as vm (vm.id)}
        <label class="flex items-center gap-2"><input type="checkbox" value={vm.id} bind:group={selected} />{vm.name ?? vm.id}</label>
      {:else}
        <p>No running sandboxes are available.</p>
      {/each}
    </fieldset>
    <button type="button" class="text-primary hover:underline disabled:opacity-50" disabled={busy || !selected.length}
      onclick={review}>{busy ? 'Loading generations…' : 'Review selected sandboxes'}</button>
  {:else if phase === 'review' && reviewed}
    <p class="break-all">Apply saved revision {revision.revision_id}.</p>
    <p>Access changes take effect while the sandbox runs and may close pending security requests. Workload changes take effect on the next start.</p>
    <ul class="space-y-2 rounded-lg border border-line-2 p-3">
      {#each reviewed.targets as target (target.session_id)}
        <li class="break-all">
          <p class="font-medium">{running.find(vm => vm.id === target.session_id)?.name ?? target.session_id}</p>
          <p>Generation {target.generation}</p>
          <p>Current desired revision {target.base.environment_id}@{target.base.revision_id}</p>
        </li>
      {/each}
    </ul>
    <label for={id} class="block font-medium">Reason for applying this revision</label>
    <input {id} bind:value={reason} disabled={busy || attempt !== null} class="w-full rounded-lg border border-line-2 bg-background-1 px-3 py-2" />
    {#if attempt && error}<p class="break-all text-xs text-muted-foreground-1">Request {attempt.request_id}</p>{/if}
    <button type="button" disabled={busy || !reason.trim()} onclick={submit}
      class="rounded-lg bg-primary px-3 py-2 font-medium text-primary-foreground hover:bg-primary-hover disabled:opacity-50">
      {busy ? 'Applying…' : attempt ? 'Check application' : 'Apply reviewed revision'}
    </button>
    {#if !attempt}<button type="button" class="px-3 py-2 text-primary hover:underline" disabled={busy}
      onclick={() => { phase = 'select'; reviewed = null; error = ''; }}>Change selection</button>{/if}
  {:else if phase === 'receipt'}
    <p role="status" class="break-all">Request {attempt?.request_id}{detail ? ` · Application ${detail.summary.application}` : ''}</p>
    {#if detail?.applications?.length}
      <ul aria-label="Sandbox application results" class="space-y-2">
        {#each detail.applications as item (item.target.session_id)}
          <li class="break-words">
            <span class="font-medium">{running.find(vm => vm.id === item.target.session_id)?.name ?? item.target.session_id}</span>
            <span> · {item.state}{item.composition?.pending_restart ? ' · Restart required' : ''}</span>
            {#if item.failure_reason}<p class="text-destructive-text">{item.failure_reason}</p>{/if}
          </li>
        {/each}
      </ul>
    {/if}
    <button type="button" class="text-primary hover:underline" onclick={readReceipt}>Refresh application status</button>
    {#if retryable.length}
      <button type="button" class="px-3 py-2 text-primary hover:underline" onclick={reviewFailedTargets}>Review failed sandboxes</button>
    {/if}
  {/if}
  {#if error}<p role="alert" class="text-destructive-text">{error}</p>{/if}
</section>
