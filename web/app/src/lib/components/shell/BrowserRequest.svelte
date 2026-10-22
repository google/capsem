<script lang="ts">
  import type { BrowserRequest } from '@capsem/sdk';
  import { browserUrl } from '../../models/browser';
  import { openUrl } from '../../api';

  let { request }: { request?: BrowserRequest | null } = $props();
  let url = $derived(browserUrl(request?.url));
  let host = $derived(url ? new URL(url).hostname : '');
  let error = $state(false);
  $effect(() => { void url; error = false; });

  function openRequestedBrowser() {
    const target = url;
    if (!target) return;
    error = false;
    void openUrl(target).catch(() => { if (url === target) error = true; });
  }
</script>

{#if url}
  <button
    type="button"
    class="max-w-64 shrink-0 truncate rounded-lg bg-primary px-3 py-1 text-sm text-primary-foreground hover:bg-primary-hover"
    title={`Open ${host} on this device`}
    onclick={openRequestedBrowser}
  >Open browser · {host}</button>
  {#if error}<span role="alert" class="text-sm text-destructive-text">Could not open browser</span>{/if}
{/if}
