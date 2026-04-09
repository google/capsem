<script lang="ts">
  import type { MockLogEntry } from '../../mock.ts';

  let levelFilter = $state<'all' | 'info' | 'warn' | 'error'>('all');
  let sourceFilter = $state<string>('all');
  let searchText = $state('');
  let autoScroll = $state(true);
  let scrollContainer: HTMLDivElement | null = $state(null);

  const entries: MockLogEntry[] = [
    { id: 'svc-01', timestamp: '2026-04-09T09:00:00.100Z', level: 'info', source: 'service', message: 'capsem-service started (v0.9.1)' },
    { id: 'svc-02', timestamp: '2026-04-09T09:00:00.200Z', level: 'info', source: 'service', message: 'Listening on /tmp/capsem.sock' },
    { id: 'svc-03', timestamp: '2026-04-09T09:00:00.300Z', level: 'info', source: 'gateway', message: 'Gateway started on 127.0.0.1:19222' },
    { id: 'svc-04', timestamp: '2026-04-09T09:00:01.000Z', level: 'info', source: 'service', message: 'Asset manifest loaded (aarch64, 3 entries)' },
    { id: 'svc-05', timestamp: '2026-04-09T10:05:30.000Z', level: 'info', source: 'service', message: 'VM provisioned: dev-sandbox (vm-1)' },
    { id: 'svc-06', timestamp: '2026-04-09T10:05:30.050Z', level: 'info', source: 'process', message: 'Spawned capsem-process for vm-1 (pid 42301)' },
    { id: 'svc-07', timestamp: '2026-04-09T10:05:33.200Z', level: 'info', source: 'service', message: 'VM running: dev-sandbox (vm-1)' },
    { id: 'svc-08', timestamp: '2026-04-09T10:10:00.000Z', level: 'info', source: 'service', message: 'VM provisioned: ci-runner (vm-2)' },
    { id: 'svc-09', timestamp: '2026-04-09T10:10:03.500Z', level: 'info', source: 'service', message: 'VM running: ci-runner (vm-2)' },
    { id: 'svc-10', timestamp: '2026-04-09T10:15:00.000Z', level: 'warn', source: 'service', message: 'VM ml-training (vm-4) exited with error: OOM killed' },
    { id: 'svc-11', timestamp: '2026-04-09T10:20:00.000Z', level: 'info', source: 'gateway', message: 'WebSocket connected for vm-1 (terminal)' },
    { id: 'svc-12', timestamp: '2026-04-09T10:25:00.000Z', level: 'error', source: 'service', message: 'Failed to resume vm-3: snapshot corrupted' },
    { id: 'svc-13', timestamp: '2026-04-09T10:30:00.000Z', level: 'info', source: 'gateway', message: 'Status cache refreshed (5 VMs)' },
  ];

  const sources = $derived([...new Set(entries.map(e => e.source))].sort());

  const filtered = $derived.by(() => {
    let result = entries;
    if (levelFilter !== 'all') result = result.filter(e => e.level === levelFilter);
    if (sourceFilter !== 'all') result = result.filter(e => e.source === sourceFilter);
    if (searchText.trim()) {
      const q = searchText.trim().toLowerCase();
      result = result.filter(e => e.message.toLowerCase().includes(q) || e.source.toLowerCase().includes(q));
    }
    return result;
  });

  $effect(() => {
    if (autoScroll && scrollContainer && filtered.length > 0) {
      scrollContainer.scrollTop = scrollContainer.scrollHeight;
    }
  });

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
  <!-- Header -->
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

    <select
      class="text-sm border border-line-2 rounded-lg bg-layer text-foreground px-2 py-1 focus:border-primary focus:ring-primary"
      bind:value={sourceFilter}
    >
      <option value="all">All sources</option>
      {#each sources as source}
        <option value={source}>{source}</option>
      {/each}
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

  <!-- Log entries -->
  <div class="flex-1 overflow-auto font-mono text-sm" bind:this={scrollContainer}>
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
              <td class="px-2 py-1.5 text-xs text-muted-foreground-1 whitespace-nowrap w-24">
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
