<script lang="ts">
  import { getBaseUrl } from '../../api';
  import { isSurfaceReconnectMessage, surfaceEmbedUrl } from '../../models/surface';
  import { vmStore } from '../../stores/vms.svelte';

  let { vmId, revision = 0 }: { vmId: string; revision?: number } = $props();
  let frame: HTMLIFrameElement | undefined = $state();
  let reconnect = $state(0);
  let src = $derived(surfaceEmbedUrl(getBaseUrl(), vmId, window.location.origin));
  // Keep the graphical connection across UI/Terminal/Stats switches. A VM
  // lifecycle transition starts a fresh bootstrap against the current owner.
  let status = $derived(vmStore.vms.find(vm => vm.id === vmId)?.status);
</script>

<svelte:window onmessage={event => {
  if (isSurfaceReconnectMessage(event, frame?.contentWindow)) reconnect += 1;
}} />

{#key `${status}:${revision}:${reconnect}`}
  <iframe
    bind:this={frame}
    {src}
    title="UI: {vmId}"
    sandbox="allow-scripts allow-same-origin allow-forms"
    referrerpolicy="no-referrer"
    class="w-full h-full border-0"
  ></iframe>
{/key}
