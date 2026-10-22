<script lang="ts">
  import type { BrowserRequest } from '@capsem/sdk';
  import { browserUrl, BrowserRequestCursor } from '../../models/browser';
  import { openUrl } from '../../api';

  let { vmId, request }: { vmId?: string; request?: BrowserRequest | null } = $props();
  let url = $derived(browserUrl(request?.url));
  let requestId = $derived(request?.id);
  let host = $derived(url ? new URL(url).hostname : '');
  let error = $state(false);
  const requests = new BrowserRequestCursor();
  $effect(() => { void vmId; void requestId; void url; error = false; });
  $effect(() => {
    const target = requests.next(vmId, request);
    // The native shell can launch the local browser after the guest round trip.
    // A browser tab still needs its explicit action because its gesture expired.
    if (target) {
      const native = '__TAURI_INTERNALS__' in window;
      console.log('[browser] fresh request vm=%s id=%s native=%s', vmId, requestId, native);
      if (native) openTarget(target, true);
    }
  });

  function openTarget(target: string, automatic = false) {
    const session = vmId;
    const id = requestId;
    error = false;
    console.log('[browser] handoff id=%s automatic=%s', id, automatic);
    void openUrl(target).then(() => {
      console.log('[browser] handoff accepted id=%s', id);
    }).catch(() => {
      console.warn('[browser] handoff failed id=%s', id);
      if (vmId === session && requestId === id && url === target) error = true;
    });
  }
</script>

{#if url}
  <button
    type="button"
    class="max-w-64 shrink-0 truncate rounded-lg bg-primary px-3 py-1 text-sm text-primary-foreground hover:bg-primary-hover"
    title={`Open ${host} on this device`}
    onclick={() => { if (url) openTarget(url); }}
  >Open browser · {host}</button>
  {#if error}<span role="alert" class="text-sm text-destructive-text">Could not open browser</span>{/if}
{/if}
