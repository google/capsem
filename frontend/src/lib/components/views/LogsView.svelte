<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { mockLogEntries } from '../../mock.ts';
  import type { MockLogEntry } from '../../mock.ts';

  let { vmId }: { vmId: string } = $props();

  // Filters
  let levelFilter = $state<'all' | 'info' | 'warn' | 'error'>('all');
  let sourceFilter = $state<string>('all');
  let searchText = $state('');
  let autoScroll = $state(true);

  // All available sources from mock data
  const sources = $derived([...new Set(mockLogEntries.map(e => e.source))].sort());

  // Filtered entries
  const filtered = $derived.by(() => {
    let entries = mockLogEntries;
    if (levelFilter !== 'all') {
      entries = entries.filter(e => e.level === levelFilter);
    }
    if (sourceFilter !== 'all') {
      entries = entries.filter(e => e.source === sourceFilter);
    }
    if (searchText.trim()) {
      const q = searchText.trim().toLowerCase();
      entries = entries.filter(e => e.message.toLowerCase().includes(q) || e.source.toLowerCase().includes(q));
    }
    return entries;
  });

  let scrollContainer: HTMLDivElement | null = $state(null);

  // Auto-scroll to bottom when entries change
  $effect(() => {
    if (autoScroll && scrollContainer && filtered.length > 0) {
      // Access filtered.length to create dependency
      scrollContainer.scrollTop = scrollContainer.scrollHeight;
    }
  });

  function formatRelativeTime(iso: string): string {
    const now = Date.now();
    const then = new Date(iso).getTime();
    const diffMs = now - then;

    if (diffMs < 0) return 'just now';
    if (diffMs < 60_000) return `${Math.floor(diffMs / 1000)}s ago`;
    if (diffMs < 3_600_000) return `${Math.floor(diffMs / 60_000)}m ago`;
    if (diffMs < 86_400_000) return `${Math.floor(diffMs / 3_600_000)}h ago`;
    return new Date(iso).toLocaleDateString();
  }

  function formatTimestamp(iso: string): string {
    return new Date(iso).toLocaleTimeString(undefined, { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit', fractionalSecondDigits: 3 });
  }

  const levelClasses: Record<string, string> = {
    info: 'bg-primary/10 text-primary',
    warn: 'bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400',
    error: 'bg-destructive/10 text-destructive',
  };
</script>

<div class="flex flex-col h-full">
  <!-- Filter bar -->
  <div class="flex items-center gap-x-3 border-b border-line-2 bg-layer px-4 py-2">
    <!-- Level filter -->
    <select
      class="text-sm border border-line-2 rounded-lg bg-layer text-foreground px-2 py-1 focus:border-primary focus:ring-primary"
      bind:value={levelFilter}
    >
      <option value="all">All levels</option>
      <option value="info">Info</option>
      <option value="warn">Warn</option>
      <option value="error">Error</option>
    </select>

    <!-- Source filter -->
    <select
      class="text-sm border border-line-2 rounded-lg bg-layer text-foreground px-2 py-1 focus:border-primary focus:ring-primary"
      bind:value={sourceFilter}
    >
      <option value="all">All sources</option>
      {#each sources as source}
        <option value={source}>{source}</option>
      {/each}
    </select>

    <!-- Text search -->
    <input
      type="text"
      class="flex-1 text-sm border border-line-2 rounded-lg bg-layer text-foreground px-3 py-1 placeholder:text-muted-foreground focus:border-primary focus:ring-primary"
      placeholder="Search logs..."
      bind:value={searchText}
    />

    <!-- Auto-scroll toggle -->
    <label class="flex items-center gap-x-1.5 text-sm text-muted-foreground-1 cursor-pointer select-none">
      <input
        type="checkbox"
        class="rounded border-line-2 text-primary focus:ring-primary"
        bind:checked={autoScroll}
      />
      Auto-scroll
    </label>

    <!-- Count -->
    <span class="text-xs text-muted-foreground">{filtered.length} entries</span>
  </div>

  <!-- Log entries -->
  <div
    class="flex-1 overflow-auto font-mono text-sm"
    bind:this={scrollContainer}
  >
    {#if filtered.length === 0}
      <div class="flex items-center justify-center h-full">
        <p class="text-muted-foreground">No log entries match filters</p>
      </div>
    {:else}
      <table class="w-full">
        <tbody>
          {#each filtered as entry}
            <tr class="border-b border-line-2 hover:bg-muted-hover">
              <td class="px-3 py-1.5 text-xs text-muted-foreground whitespace-nowrap w-24" title={entry.timestamp}>
                {formatTimestamp(entry.timestamp)}
              </td>
              <td class="px-2 py-1.5 w-16">
                <span class="inline-flex items-center px-1.5 py-0.5 rounded text-xs font-medium uppercase {levelClasses[entry.level]}">
                  {entry.level}
                </span>
              </td>
              <td class="px-2 py-1.5 text-xs text-muted-foreground-1 whitespace-nowrap w-32">
                {entry.source}
              </td>
              <td class="px-3 py-1.5 text-foreground">
                {entry.message}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </div>
</div>
