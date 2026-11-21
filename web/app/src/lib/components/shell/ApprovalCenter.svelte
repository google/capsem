<script lang="ts">
  import { onMount } from 'svelte';
  import { SecurityAskStatus, type RequestSummary, type SecurityAskDecision } from '@capsem/sdk';
  import { listEnvironmentRequests, listSecurityAsks, onApprovalQueueChanged, resolveSecurityAsk } from '../../api';
  import { ApiError } from '../../gateway-sdk';
  import { vmStore } from '../../stores/vms.svelte';
  import { watchVisibleRefresh } from '../../models/visible-refresh';
  import { ApprovalQueue, ApprovalNotices, ApprovalRefresh, type ApprovalState } from '../../models/security-asks';
  import ApprovalPanel from './ApprovalPanel.svelte';
  import EnvironmentApprovalPanel from './EnvironmentApprovalPanel.svelte';

  let activeVm = $state<string | null>(null);
  let focusAsk = $state<string | null>(null);
  let view = $state<ApprovalState>({ response: null, loading: false, resolving: null, error: null });
  let notice = $state<{ vmId: string; askId: string; generation: string } | null>(null);
  let queueCounts = $state(new Map<string, number>());
  let environmentRequests = $state<RequestSummary[]>([]);
  let environmentOpen = $state(false);
  let environmentEpoch = 0;
  const notices = new ApprovalNotices();
  const queue = new ApprovalQueue(listSecurityAsks, resolveSecurityAsk, value => view = value);
  let names = $derived(new Map(vmStore.vms.map(vm => [vm.id, vm.name])));
  let pending = $derived.by(() => {
    const counts = new Map(vmStore.vms.map(vm => [vm.id, vm.pending_approval_count ?? 0]));
    for (const [id, total] of queueCounts) counts.set(id, total);
    return Array.from(counts).filter(([, total]) => total > 0);
  });
  let count = $derived(pending.reduce((total, [, pendingCount]) => total + pendingCount, 0));
  let activeName = $derived(activeVm ? names.get(activeVm) ?? activeVm : 'Session');
  let noticeLabel = $derived(notice ? `Approval requested by ${names.get(notice.vmId) ?? notice.vmId}` : '');

  function open(vmId: string, askId: string | null = null) {
    environmentOpen = false;
    activeVm = vmId;
    focusAsk = askId;
    notice = null;
    void queue.load(vmId);
  }
  function openEnvironment() {
    close();
    environmentOpen = true;
  }

  async function refreshEnvironment() {
    const epoch = ++environmentEpoch;
    try {
      const listed = await listEnvironmentRequests();
      if (epoch === environmentEpoch) environmentRequests = listed;
    } catch {
      // Keep the last confirmed queue while the service reconnects.
    }
  }
  function close() {
    activeVm = null;
    focusAsk = null;
    queue.dispose();
  }

  const refresh = new ApprovalRefresh(
    async id => {
      try {
        return await listSecurityAsks(id);
      } catch (error) {
        if (error instanceof ApiError && error.status === 404) return null;
        throw error;
      }
    },
    (id, list) => {
      queueCounts = new Map(queueCounts).set(id, list?.pending_count ?? 0);
      const currentNotice = notice;
      if (currentNotice?.vmId === id && (list?.generation !== currentNotice.generation || !list.requests.some(
        request => request.ask_id === currentNotice.askId && request.status === SecurityAskStatus.PENDING
          && request.expires_at_unix_ms > Date.now()
      ))) notice = null;
      if (!list) {
        if (activeVm === id) close();
        return;
      }
      if (activeVm === id) queue.synchronize(list);
      const first = notices.observe(list)[0];
      if (first && activeVm !== id) notice = { vmId: id, askId: first, generation: list.generation };
    },
    id => activeVm === id && Boolean(view.resolving),
  );

  function reconcile(changed = false): Promise<void> {
    const ids = new Set(pending.map(([id]) => id));
    // A status snapshot can discover a request missed while the socket was down.
    for (const vm of vmStore.vms) {
      if ((vm.pending_approval_count ?? 0) > 0) ids.add(vm.id);
    }
    if (activeVm) ids.add(activeVm);
    const retained = new Map([...queueCounts].filter(([id, total]) => total > 0 || names.has(id)));
    if (retained.size !== queueCounts.size) queueCounts = retained;
    return refresh.enqueue(ids, changed);
  }

  async function decide(askId: string, decision: SecurityAskDecision): Promise<void> {
    const id = activeVm;
    if (!id) return;
    await queue.decide(askId, decision);
    await refresh.enqueue([id], true);
  }

  onMount(() => {
    const unsubscribe = onApprovalQueueChanged(
      id => { void refresh.enqueue([id], true); void refreshEnvironment(); },
      () => { void vmStore.refresh().then(() => reconcile(true)); void refreshEnvironment(); },
    );
    void reconcile();
    void refreshEnvironment();
    const stop = watchVisibleRefresh(async () => { await Promise.all([reconcile(), refreshEnvironment()]); }, 2000);
    return () => {
      unsubscribe();
      stop();
      refresh.dispose();
      queue.dispose();
      environmentEpoch++;
    };
  });
</script>
{#if count>0 || activeVm || environmentRequests.length>0}
  <div class="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-line-2 bg-background-1 px-4 py-2 text-sm">
    <div class="flex flex-wrap items-center gap-2">
      {#if count>0 || activeVm}<span class="font-medium text-primary">{count} pending {count===1?'approval':'approvals'}</span>{/if}
      {#each pending as [id] (id)}<button type="button" class="rounded-md px-2 py-1 text-primary hover:bg-muted-hover" onclick={()=>open(id)}>Review {names.get(id)??id}</button>{/each}
      {#if environmentRequests.length>0}
        <button type="button" class="rounded-md px-2 py-1 text-primary hover:bg-muted-hover" onclick={openEnvironment}>
          Review environment changes ({environmentRequests.length})
        </button>
      {/if}
    </div>
    {#if notice}
      <button type="button" class="text-primary hover:underline" aria-label={noticeLabel} onclick={() => notice && open(notice.vmId, notice.askId)}>
        <span role="status">{noticeLabel}</span>
      </button>
    {/if}
  </div>
{/if}
{#if activeVm}<ApprovalPanel state={view} vmName={activeName} {focusAsk} onclose={close} onretry={()=>{if(activeVm)void queue.load(activeVm);}} ondecide={decide}/>{/if}
{#if environmentOpen}
  <EnvironmentApprovalPanel requests={environmentRequests} {names}
    onclose={() => environmentOpen = false} onresolved={refreshEnvironment} />
{/if}
