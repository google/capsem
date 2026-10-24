<script lang="ts">
  import type { TerminalSummary } from '@capsem/sdk';
  import Bell from 'phosphor-svelte/lib/Bell';

  let { terminal }: { terminal?: TerminalSummary | null } = $props();
  // Guest metadata is text, never markup or a full-value tooltip. Notifications
  // are last observed, and bells are cumulative; neither means unread or busy.
  function label(value: string): string {
    let result = '';
    let length = 0;
    for (const character of value) {
      if (/[\u0000-\u001f\u007f-\u009f]/.test(character)) continue;
      result += character;
      if (++length === 120) break;
    }
    return result;
  }
  const title = $derived(label(terminal?.title ?? ''));
  const notification = $derived(label(terminal?.last_notification?.title ?? ''));
</script>

{#if title || terminal?.last_notification || (terminal?.bell_count ?? 0) > 0}
  <div class="max-w-xs text-xs font-normal text-muted-foreground-1">
    {#if title}<p class="truncate" aria-label="Terminal title">{title}</p>{/if}
    {#if terminal?.last_notification}
      <p class="truncate text-primary" aria-label="Latest notification">{notification || 'Terminal notification'}</p>
    {/if}
    {#if terminal && terminal.bell_count > 0}
      <span class="inline-flex items-center gap-1 text-primary"><Bell size={12} aria-hidden="true" />Terminal bells: {terminal.bell_count}</span>
    {/if}
  </div>
{/if}
