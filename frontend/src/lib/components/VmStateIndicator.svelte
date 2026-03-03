<script lang="ts">
  import { vmStore } from '../stores/vm.svelte';

  let { collapsed = false }: { collapsed?: boolean } = $props();
</script>

<div class="flex items-center gap-1.5" title={collapsed ? vmStore.vmState : undefined}>
  <span class="inline-block size-2 rounded-full" class:bg-allowed={vmStore.vmState === 'running'} class:bg-denied={vmStore.vmState === 'stopped' || vmStore.vmState === 'error'} class:bg-caution={vmStore.vmState === 'booting'} class:bg-base-content={vmStore.vmState === 'not created'}></span>
  {#if !collapsed}
    <span class="text-xs whitespace-nowrap" class:text-allowed={vmStore.vmState === 'running'} class:text-denied={vmStore.vmState === 'stopped' || vmStore.vmState === 'error'} class:text-caution={vmStore.vmState === 'booting'}>{vmStore.vmState}</span>
  {/if}
</div>
