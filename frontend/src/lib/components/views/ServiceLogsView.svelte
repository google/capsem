<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import * as api from '../../api';

  interface LogEntry {
    timestamp: string;
    level: string;
    message: string;
    source: string;
  }

  let entries = $state<LogEntry[]>([]);
  let pollInterval: ReturnType<typeof setInterval> | null = null;

  onMount(async () => {
    await fetchLogs();
    pollInterval = setInterval(fetchLogs, 5000);
  });

  onDestroy(() => {
    if (pollInterval) clearInterval(pollInterval);
  });

  function parseNdjson(text: string): LogEntry[] {
    if (!text) return [];
    return text.split('\n').filter(l => l.trim()).map(line => {
      try {
        const obj = JSON.parse(line);
        return {
          timestamp: obj.timestamp ?? '',
          level: (obj.level ?? 'info').toLowerCase(),
          message: obj.fields?.message ?? JSON.stringify(obj.fields ?? {}),
          source: obj.target ?? 'unknown',
        };
      } catch {
        return { timestamp: '', level: 'info', message: line, source: 'raw' };
      }
    });
  }

  async function fetchLogs() {
    if (!api.isConnected()) return;
    try {
      const text = await api.getServiceLogs();
      if (text) entries = parseNdjson(text);
    } catch {
      // Keep existing data
    }
  }

  let levelFilter = $state<'all' | 'info' | 'warn' | 'error'>('all');
  let searchText = $state('');
  let autoScroll = $state(true);

  const filtered = $derived.by(() => {
    let result = entries;
    if (levelFilter !== 'all') result = result.filter(e => e.level === levelFilter);
    if (searchText.trim()) {
      const q = searchText.trim().toLowerCase();
      result = result.filter(e => e.message.toLowerCase().includes(q) || e.source.toLowerCase().includes(q));
    }
    return result;
  });

  let scrollContainer: HTMLDivElement | null = $state(null);

  $effect(() => {
    if (autoScroll && scrollContainer && filtered.length > 0) {
      scrollContainer.scrollTop = scrollContainer.scrollHeight;
    }
  });

  function formatTimestamp(iso: string): string {
    if (!iso) return '';
    return new Date(iso).toLocaleTimeString(undefined, { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit', fractionalSecondDigits: 3 });
  }

  const levelClasses: Record<string, string> = {
    info: 'bg-primary/10 text-primary',
    warn: 'bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400',
    error: 'bg-destructive/10 text-destructive',
  };
</script>

<div class="flex flex-col h-full">
  <div class="flex items-center gap-x-3 border-b border-line-2 bg-layer px-4 py-2">
    <span class="text-sm font-medium text-foreground">Service Logs</span>
    <div class="h-4 border-l border-line-2"></div>

    <select
      class="text-sm border border-line-2 rounded-lg bg-layer text-foreground px-2 py-1 focus:border-primary focus:ring-primary"
      bind:value={levelFilter}
    >
      <option value="all">All levels</option>
      <option value="info">Info</option>
      <option value="warn">Warn</option>
      <option value="error">Error</option>
    </select>

    <input
      type="text"
      class="flex-1 text-sm border border-line-2 rounded-lg bg-layer text-foreground px-3 py-1 placeholder:text-muted-foreground focus:border-primary focus:ring-primary"
      placeholder="Search logs..."
      bind:value={searchText}
    />

    <label class="flex items-center gap-x-1.5 text-sm text-muted-foreground-1 cursor-pointer select-none">
      <input type="checkbox" class="rounded border-line-2 text-primary focus:ring-primary" bind:checked={autoScroll} />
      Auto-scroll
    </label>

    <span class="text-xs text-muted-foreground">{filtered.length} entries</span>
  </div>

  <div class="flex-1 overflow-auto font-mono text-sm" bind:this={scrollContainer}>
    {#if filtered.length === 0}
      <div class="flex items-center justify-center h-full">
        <p class="text-muted-foreground">No log entries{entries.length > 0 ? ' match filters' : ''}</p>
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
                <span class="inline-flex items-center px-1.5 py-0.5 rounded text-xs font-medium uppercase {levelClasses[entry.level] ?? levelClasses.info}">
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
