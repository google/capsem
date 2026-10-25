<script lang="ts">
  import {
    InteractionMessageKind,
    InteractionRequestKind,
    InteractionToolCallKind,
    InteractionToolResultKind,
    type InteractionReport,
    type Interaction,
  } from '@capsem/sdk';
  import { formatTime } from '../../../format';
  import {
    displayInteractions,
    interactionLabel,
    type InteractionScope,
  } from '../../../interaction-viewer';
  import StatsBadge from './StatsBadge.svelte';
  import InteractionPayload from './InteractionPayload.svelte';

  let {
    report,
    scope,
    title,
    onDetails,
  }: {
    report: InteractionReport;
    scope: InteractionScope;
    title: string;
    onDetails: (item: Interaction) => void;
  } = $props();

  const items = $derived(displayInteractions(report, scope));
</script>

<section class="mb-6">
  <div class="mb-2 flex items-center justify-between gap-3">
    <h3 class="text-sm font-semibold text-foreground">{title}</h3>
    <span class="text-xs text-muted-foreground">{items.length} items</span>
  </div>
  <div class="space-y-2">
    {#each items as item (`${item.event_id}:${item.item_index ?? ''}`)}
      <article class="rounded-xl border border-card-line bg-card p-4">
        <header class="mb-3 flex flex-wrap items-center gap-x-3 gap-y-1">
          <span class="text-sm font-medium text-foreground">{interactionLabel(item)}</span>
          <span class="text-xs text-muted-foreground">{formatTime(item.timestamp)}</span>
          <button type="button" class="ml-auto text-xs text-info hover:underline" onclick={() => onDetails(item)}>Details</button>
        </header>

        <div class="space-y-2">
          {#if item.content.kind === InteractionRequestKind.REQUEST_PREVIEW}
            <p class="text-xs text-muted-foreground">Request captured. Open Details to inspect the payload.</p>
          {:else if item.content.kind === InteractionMessageKind.MESSAGE}
            {#each item.content.blocks as block, index (`${block.kind}:${index}`)}
              <InteractionPayload label={block.kind} payload={block.payload} />
            {/each}
          {:else if item.content.kind === InteractionToolCallKind.TOOL_CALL}
            <div class="flex flex-wrap items-center gap-2 text-xs">
              <span class="font-mono font-semibold text-foreground">{item.content.tool_name}</span>
              <span class="text-muted-foreground-1">{item.content.server_name ?? 'local'}</span>
              <span class="text-muted-foreground-1">{item.content.origin}</span>
              <StatsBadge value={item.content.decision} kind="decision" />
              <span class="font-mono text-[10px] text-muted-foreground">{item.content.call_id}</span>
            </div>
            {#if item.content.result}
              <p class="text-xs text-muted-foreground">{item.content.result.is_error ? 'Tool returned an error' : 'Result received'}</p>
              {#if item.content.result.error_message}
                <p class="text-xs text-destructive">{item.content.result.error_message}</p>
              {/if}
            {/if}
          {:else if item.content.kind === InteractionToolResultKind.TOOL_RESULT}
            <div class="flex flex-wrap items-center gap-2 text-xs">
              <span class="font-mono text-muted-foreground-1">{item.content.call_id}</span>
              {#if item.content.is_error}
                <span class="rounded bg-destructive/10 px-1.5 py-0.5 text-destructive">error</span>
              {/if}
            </div>
            <p class="text-xs text-muted-foreground">Result captured. Open Details to inspect the payload.</p>
            {#if item.content.error_message}
              <p class="text-xs text-destructive">{item.content.error_message}</p>
            {/if}
          {/if}

        </div>
      </article>
    {:else}
      <div class="rounded-xl border border-card-line bg-card px-4 py-8 text-center text-sm text-muted-foreground">
        No interactions
      </div>
    {/each}
  </div>
</section>
