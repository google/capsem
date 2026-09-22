<script lang="ts">
  import type { CapturedPayload } from '@capsem/sdk';
  import { payloadDisplay } from '../../../interaction-viewer';

  let {
    label,
    payload,
  }: {
    label: string;
    payload?: CapturedPayload | null;
  } = $props();

  const display = $derived(payloadDisplay(payload));
</script>

{#if display}
  <div class="rounded-lg border border-card-line bg-background-1 p-3">
    <div class="mb-2 flex items-center gap-2 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
      <span>{label}</span>
      <span class="rounded bg-muted px-1.5 py-0.5 text-muted-foreground-1">{display.status}</span>
      {#if display.reason}
        <span class="rounded bg-destructive/10 px-1.5 py-0.5 text-destructive">{display.reason}</span>
      {/if}
    </div>
    <pre class="max-h-64 overflow-auto whitespace-pre-wrap break-words font-mono text-xs leading-5 text-foreground">{display.text}</pre>
  </div>
{/if}
