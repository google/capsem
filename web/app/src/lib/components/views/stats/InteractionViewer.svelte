<script lang="ts">
  import {
    InteractionMessageKind,
    InteractionRequestKind,
    InteractionToolCallKind,
    InteractionToolResultKind,
    type InteractionReport,
  } from '@capsem/sdk';
  import { formatBytes, formatTime } from '../../../format';
  import {
    interactionBodies,
    interactionItems,
    interactionLabel,
    type InteractionScope,
  } from '../../../interaction-viewer';
  import StatsBadge from './StatsBadge.svelte';
  import InteractionPayload from './InteractionPayload.svelte';

  let {
    report,
    scope,
    title,
  }: {
    report: InteractionReport;
    scope: InteractionScope;
    title: string;
  } = $props();

  const items = $derived(interactionItems(report, scope));
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
          {#if item.trace_id}
            <span class="font-mono text-[10px] text-muted-foreground-1">trace {item.trace_id}</span>
          {/if}
          {#if item.turn_id}
            <span class="font-mono text-[10px] text-muted-foreground-1">turn {item.turn_id}</span>
          {/if}
        </header>

        <div class="space-y-2">
          {#if item.content.kind === InteractionRequestKind.REQUEST_PREVIEW}
            <InteractionPayload label="Request" payload={item.content.payload} />
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
            <InteractionPayload label="Arguments" payload={item.content.arguments} />
            <InteractionPayload label="Observed request" payload={item.content.request} />
            {#if item.content.result}
              <InteractionPayload label="Result" payload={item.content.result.payload} />
              <InteractionPayload label="Observed response" payload={item.content.result.response} />
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
            <InteractionPayload label="Result" payload={item.content.payload} />
            <InteractionPayload label="Observed response" payload={item.content.response} />
            {#if item.content.error_message}
              <p class="text-xs text-destructive">{item.content.error_message}</p>
            {/if}
          {/if}

          {#each interactionBodies(report, item.event_id) as body (`${body.direction}:${body.body_hash}`)}
            <div class="space-y-1">
              <div class="flex flex-wrap gap-2 text-[10px] text-muted-foreground-1">
                <span>{body.direction} body</span>
                {#if body.content_type}<span>{body.content_type}</span>{/if}
                <span>{formatBytes(body.stored_bytes)} of {formatBytes(body.original_bytes)}</span>
                <span class="font-mono">{body.body_hash}</span>
              </div>
              <InteractionPayload label={`${body.direction} body`} payload={body.payload} />
            </div>
          {/each}
        </div>
      </article>
    {:else}
      <div class="rounded-xl border border-card-line bg-card px-4 py-8 text-center text-sm text-muted-foreground">
        No interactions
      </div>
    {/each}
  </div>
</section>
