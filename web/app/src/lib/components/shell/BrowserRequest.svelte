<script lang="ts">
  import type { BrowserRequest } from '@capsem/sdk';
  import { browserUrl } from '../../models/browser';
  import { BrowserHandoff } from '../../models/browser-handoff.svelte';
  import { acknowledgeBrowserRequest, openUrl } from '../../api';

  let { vmId, request }: { vmId?: string; request?: BrowserRequest | null } = $props();
  let url = $derived(browserUrl(request?.url));
  let host = $derived(url ? new URL(url).hostname : '');
  const handoff = new BrowserHandoff();
</script>

{#if handoff.pending(vmId, request)}
  <button
    type="button"
    class="max-w-64 shrink-0 truncate rounded-lg bg-primary px-3 py-1 text-sm text-primary-foreground hover:bg-primary-hover"
    title={`Open ${url} on this device`}
    onclick={() => { void handoff.open(vmId, request, openUrl, acknowledgeBrowserRequest); }}
  >Open browser · {host}</button>
  {#if handoff.failed(vmId, request)}<span role="alert" class="text-sm text-destructive-text">Could not open browser</span>{/if}
{/if}

{#if handoff.confirmationFailed(vmId, request)}
  <span role="alert" class="text-sm text-muted-foreground">Browser opened; confirmation unavailable</span>
  <button type="button" class="shrink-0 text-sm text-primary hover:underline"
    onclick={() => { void handoff.retryConfirmation(vmId, request, acknowledgeBrowserRequest); }}>Retry confirmation</button>
{/if}
