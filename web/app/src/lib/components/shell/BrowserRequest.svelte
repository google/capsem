<script lang="ts">
  import type { BrowserRequest } from '@capsem/sdk';
  import { browserUrl, BrowserRequestCursor } from '../../models/browser';
  import { BrowserHandoff } from '../../models/browser-handoff.svelte';
  import { openUrl } from '../../api';

  let { vmId, request }: { vmId?: string; request?: BrowserRequest | null } = $props();
  let url = $derived(browserUrl(request?.url));
  let host = $derived(url ? new URL(url).hostname : '');
  const native = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
  const handoff = new BrowserHandoff();
  const requests = new BrowserRequestCursor();
  $effect(() => {
    const target = requests.next(vmId, request);
    // The native shell can launch the local browser after the guest round trip.
    // A browser tab still needs its explicit action because its gesture expired.
    if (target) {
      if (native) void handoff.open(vmId, request, openUrl);
    }
  });
</script>

{#if handoff.pending(vmId, request) && (!native || handoff.failed(vmId, request))}
  <button
    type="button"
    class="max-w-64 shrink-0 truncate rounded-lg bg-primary px-3 py-1 text-sm text-primary-foreground hover:bg-primary-hover"
    title={`Open ${host} on this device`}
    onclick={() => { void handoff.open(vmId, request, openUrl); }}
  >Open browser · {host}</button>
  {#if handoff.failed(vmId, request)}<span role="alert" class="text-sm text-destructive-text">Could not open browser</span>{/if}
{/if}
