<script lang="ts">
  import VmStateIndicator from './VmStateIndicator.svelte';
  import { vmStore } from '../stores/vm.svelte';
</script>

<footer class="flex flex-shrink-0 items-center justify-between border-t border-base-300 bg-base-200 px-3 py-1 text-xs text-base-content/60">
  <div class="flex items-center gap-2">
    <VmStateIndicator />
    {#if vmStore.terminalRenderer}
      <span class="text-base-content/40">{vmStore.terminalRenderer === 'webgl' ? 'WebGL' : 'Canvas'}</span>
    {/if}
  </div>
</footer>
