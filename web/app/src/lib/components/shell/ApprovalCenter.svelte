<script lang="ts">
  import {onMount} from 'svelte';
  import {listSecurityAsks,resolveSecurityAsk} from '../../api';
  import {vmStore} from '../../stores/vms.svelte';
  import {watchVisibleRefresh} from '../../models/visible-refresh';
  import {ApprovalQueue,ApprovalNotices,type ApprovalState} from '../../models/security-asks';
  import ApprovalPanel from './ApprovalPanel.svelte';

  let activeVm=$state<string|null>(null);
  let focusAsk=$state<string|null>(null);
  let view=$state<ApprovalState>({response:null,loading:false,resolving:null,error:null});
  let notice=$state<{vmId:string;askId:string;name:string}|null>(null);
  const notices=new ApprovalNotices();
  const queue=new ApprovalQueue(listSecurityAsks,resolveSecurityAsk,value=>view=value);
  let pending=$derived(vmStore.vms.filter(vm=>(vm.pending_approval_count??0)>0));
  let count=$derived(pending.reduce((total,vm)=>total+(vm.pending_approval_count??0),0));
  let activeName=$derived(vmStore.vms.find(vm=>vm.id===activeVm)?.name??activeVm??'Session');

  function open(vmId:string,askId:string|null=null) {
    activeVm=vmId;focusAsk=askId;notice=null;void queue.load(vmId);
  }
  function close(){activeVm=null;focusAsk=null;queue.dispose();}
  onMount(()=>{
    let stopped=false;
    let refreshing=false;
    const refresh=async()=>{
      if(refreshing || view.resolving)return;
      refreshing=true;
      try {
        if(activeVm)await queue.load(activeVm);
        for(const vm of pending) {
          if(stopped)return;
          try {
            const list=vm.id===activeVm?view.response:await listSecurityAsks(vm.id);
            if(stopped || !list || list.vm_id!==vm.id)continue;
            const first=notices.observe(list)[0];
            if(first && activeVm!==vm.id)notice={vmId:vm.id,askId:first,name:vm.name??vm.id};
          } catch {/* The polled pending count keeps the review action available. */}
        }
      } finally {refreshing=false;}
    };
    void refresh();const stop=watchVisibleRefresh(refresh,2000);
    return()=>{stopped=true;stop();queue.dispose();};
  });
</script>
{#if count>0 || activeVm}
  <div class="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-line-2 bg-background-1 px-4 py-2 text-sm">
    <div class="flex flex-wrap items-center gap-2">
      <span class="font-medium text-primary">{count} pending {count===1?'approval':'approvals'}</span>
      {#each pending as vm (vm.id)}<button type="button" class="rounded-md px-2 py-1 text-primary hover:bg-muted-hover" onclick={()=>open(vm.id)}>Review {vm.name??vm.id}</button>{/each}
    </div>
    {#if notice}<button type="button" class="text-primary hover:underline" onclick={()=>open(notice!.vmId,notice!.askId)}><span role="status">Approval requested by {notice.name}</span></button>{/if}
  </div>
{/if}
{#if activeVm}<ApprovalPanel state={view} vmName={activeName} {focusAsk} onclose={close} onretry={()=>{if(activeVm)void queue.load(activeVm);}} ondecide={(id,decision)=>{void queue.decide(id,decision).then(()=>vmStore.refresh());}}/>{/if}
