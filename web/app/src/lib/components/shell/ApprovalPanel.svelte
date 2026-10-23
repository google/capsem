<script lang="ts">
  import {onMount} from 'svelte';
  import {SecurityAskDecision,SecurityAskStatus} from '@capsem/sdk';
  import type {ApprovalState} from '../../models/security-asks';
  import X from 'phosphor-svelte/lib/X';
  let {state,vmName,focusAsk=null,onclose,onretry,ondecide}:{state:ApprovalState;vmName:string;focusAsk?:string|null;
    onclose:()=>void;onretry:()=>void;ondecide:(id:string,decision:SecurityAskDecision)=>void}=$props();
  let dialog:HTMLDialogElement;
  onMount(()=>{dialog.showModal();});
</script>
<dialog bind:this={dialog} onclose={onclose} aria-labelledby="approval-title"
  class="fixed inset-y-0 start-auto end-0 m-0 h-dvh max-h-none w-full max-w-lg overflow-y-auto border-s border-overlay-border bg-overlay text-foreground p-0 shadow-xl backdrop:bg-overlay/70">
  <div class="flex items-center gap-3 border-b border-overlay-border p-4">
    <div class="min-w-0 flex-1"><h2 id="approval-title" class="text-lg font-medium">Approval requests</h2><p class="truncate text-sm text-muted-foreground-1">{vmName}</p></div>
    <button type="button" aria-label="Close approval requests" class="rounded-lg p-2 hover:bg-muted-hover" onclick={()=>dialog.close()}><X size={18}/></button>
  </div>
  <div class="space-y-4 p-5 text-sm">
    <p class="text-muted-foreground-1">Approve or deny the original waiting operation. Decisions apply only to this session's current generation.</p>
    {#if state.error}<p role="alert" class="text-destructive">{state.error}</p>{/if}
    <button type="button" class="text-primary hover:underline disabled:opacity-50" disabled={state.loading || !!state.resolving} onclick={onretry}>{state.loading?'Checking requests…':'Check requests'}</button>
    {#if state.response}
      {#if state.response.requests.length===0}<p>No approval requests recorded for this session.</p>{/if}
      {#each state.response.requests as request (request.ask_id)}
        {@const pending=request.status===SecurityAskStatus.PENDING && request.expires_at_unix_ms>Date.now()}
        {@const statusTone=request.status===SecurityAskStatus.DENIED?'text-destructive-text':
          request.status===SecurityAskStatus.EXPIRED || request.status===SecurityAskStatus.CANCELLED || (request.status===SecurityAskStatus.PENDING && !pending)?'text-muted-foreground-1':'text-primary'}
        <section aria-label="Approval request" class="space-y-3 rounded-lg border p-4 {focusAsk===request.ask_id?'border-primary':'border-card-line'}">
          <div class="flex items-start justify-between gap-3"><h3 class="break-words font-medium">{request.rule_name}</h3><span class="shrink-0 text-xs {statusTone}">{request.status===SecurityAskStatus.PENDING && !pending?'expired':request.status}</span></div>
          <p class="break-words whitespace-pre-wrap">{request.summary}</p>
          <p class="text-xs text-muted-foreground-1">{request.event_type} · Expires {new Date(request.expires_at_unix_ms).toLocaleString()}</p>
          {#if request.resolution_reason}<p class="text-xs text-muted-foreground-1">{request.resolution_reason}</p>{/if}
          {#if pending}
            <div class="flex gap-2">
              <button type="button" class="rounded-lg bg-primary px-3 py-2 text-primary-foreground disabled:opacity-50" disabled={!!state.resolving || state.loading} onclick={()=>ondecide(request.ask_id,SecurityAskDecision.APPROVE)}>Approve once</button>
              <button type="button" class="rounded-lg border border-line-2 px-3 py-2 hover:bg-muted-hover disabled:opacity-50" disabled={!!state.resolving || state.loading} onclick={()=>ondecide(request.ask_id,SecurityAskDecision.DENY)}>Deny</button>
            </div>
          {/if}
        </section>
      {/each}
    {/if}
  </div>
</dialog>
