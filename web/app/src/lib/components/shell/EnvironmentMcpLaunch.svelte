<script lang="ts">
  import type { McpLaunchDefinition } from '../../models/environment-mcp-review';

  let { definition }: { definition: McpLaunchDefinition } = $props();
</script>

<dl class="space-y-2 text-xs">
  {#if definition.kind === 'remote'}
    <div>
      <dt class="font-medium">Remote endpoint</dt>
      <dd class="mt-1 break-all rounded-md bg-muted p-2 font-mono">{definition.url}</dd>
    </div>
  {:else}
    <div>
      <dt class="font-medium">Sandbox command and arguments</dt>
      <dd><pre class="mt-1 whitespace-pre-wrap break-words rounded-md bg-muted p-2">{JSON.stringify([definition.executable, ...(definition.args ?? [])], null, 2)}</pre></dd>
    </div>
    {#if definition.workdir}
      <div>
        <dt class="font-medium">Working directory</dt>
        <dd class="mt-1 break-all font-mono">{definition.workdir}</dd>
      </div>
    {/if}
  {/if}
</dl>
