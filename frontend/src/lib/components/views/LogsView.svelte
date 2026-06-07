<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import * as api from '../../api';

  let { vmId }: { vmId: string } = $props();

  interface LogEntry {
    timestamp: string;
    level: string;
    message: string;
    source: string;
  }

  let entries = $state<LogEntry[]>([]);
  let activeSource = $state<'process' | 'serial'>('process');
  let serialText = $state('');
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
    return text.split('\n').filter(l => l.trim()).map((line, i) => {
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
      const raw = await api.getVmLogs(vmId);
      if (raw.process_logs) {
        entries = parseNdjson(raw.process_logs);
      }
      if (raw.serial_logs) {
        serialText = raw.serial_logs;
      }
    } catch {
      // Keep existing data on error
    }
  }

  // Filters
  let levelFilter = $state<'all' | 'info' | 'warn' | 'error'>('all');
  let searchText = $state('');
  let autoScroll = $state(true);

  const filtered = $derived.by(() => {
    let result = entries;
    if (levelFilter !== 'all') {
      result = result.filter(e => e.level === levelFilter);
    }
    if (searchText.trim()) {
      const q = searchText.trim().toLowerCase();
      result = result.filter(e => e.message.toLowerCase().includes(q) || e.source.toLowerCase().includes(q));
    }
    return result;
  });

  let scrollContainer: HTMLDivElement | null = $state(null);

  $effect(() => {
    if (autoScroll && scrollContainer && (filtered.length > 0 || serialText)) {
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
  <!-- Filter bar -->
  <div class="flex items-center gap-x-3 border-b border-line-2 bg-layer px-4 py-2">
    <!-- Source toggle -->
    <div class="flex items-center bg-background-1 rounded-lg p-0.5">
      <button
        type="button"
        class="px-2.5 py-1 text-xs rounded-md transition-colors {activeSource === 'process' ? 'bg-layer text-foreground shadow-sm' : 'text-muted-foreground-1 hover:text-foreground'}"
        onclick={() => activeSource = 'process'}
      >Process</button>
      <button
        type="button"
        class="px-2.5 py-1 text-xs rounded-md transition-colors {activeSource === 'serial' ? 'bg-layer text-foreground shadow-sm' : 'text-muted-foreground-1 hover:text-foreground'}"
        onclick={() => activeSource = 'serial'}
      >Serial</button>
    </div>

    {#if activeSource === 'process'}
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

      <span class="text-xs text-muted-foreground">{filtered.length} entries</span>
    {/if}

    <div class="flex-1"></div>

    <label class="flex items-center gap-x-1.5 text-sm text-muted-foreground-1 cursor-pointer select-none">
      <input type="checkbox" class="rounded border-line-2 text-primary focus:ring-primary" bind:checked={autoScroll} />
      Auto-scroll
    </label>
  </div>

  <!-- Log entries -->
  <div class="flex-1 overflow-auto font-mono text-sm" bind:this={scrollContainer}>
    {#if activeSource === 'process'}
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
    {:else}
      {#if serialText}
        <pre class="px-4 py-3 text-foreground text-xs leading-relaxed whitespace-pre-wrap">{serialText}</pre>
      {:else}
        <div class="flex items-center justify-center h-full">
          <p class="text-muted-foreground">No serial output</p>
        </div>
      {/if}
    {/if}
  </div>
</div>
