<script lang="ts">
  import { vmStore } from '../stores/vm.svelte';

  let { collapsed = false }: { collapsed?: boolean } = $props();
</script>

<div class="flex items-center gap-1.5" title={collapsed ? vmStore.vmState : undefined}>
  <span class="inline-block size-2 rounded-full" class:bg-info={vmStore.vmState === 'running'} class:bg-secondary={vmStore.vmState === 'stopped' || vmStore.vmState === 'error'} class:bg-warning={vmStore.vmState === 'booting'} class:bg-base-content={vmStore.vmState === 'not created'}></span>
  {#if !collapsed}
    <span class="text-xs whitespace-nowrap" class:text-info={vmStore.vmState === 'running'} class:text-secondary={vmStore.vmState === 'stopped' || vmStore.vmState === 'error'} class:text-warning={vmStore.vmState === 'booting'}>{vmStore.vmState}</span>
  {/if}
</div>
