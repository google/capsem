<script lang="ts">
  import { onMount } from 'svelte';
  import * as api from '../../api';
  import { SNAPSHOT_STATS_SQL, SNAPSHOT_LIST_SQL } from '../../sql';
  import { queryDb, queryOne, queryAll } from '../../db';
  import type { ModelStats, ToolCallStat, NetworkEvent, FileEvent } from '../../types';
  import { formatDuration, formatBytes, formatTime, truncate, fmtAge } from '../../format';
  import Brain from 'phosphor-svelte/lib/Brain';
  import Wrench from 'phosphor-svelte/lib/Wrench';
  import Globe from 'phosphor-svelte/lib/Globe';
  import FileText from 'phosphor-svelte/lib/FileText';
  import ClockCounterClockwise from 'phosphor-svelte/lib/ClockCounterClockwise';

  let { vmId }: { vmId: string } = $props();

  /** Convert {columns, rows: any[][]} to an array of keyed objects. */
  function toObjects(resp: { columns: string[]; rows: any[] }): Record<string, any>[] {
    return resp.rows.map((row: any) => {
      if (Array.isArray(row)) {
        const obj: Record<string, any> = {};
        resp.columns.forEach((col, i) => { obj[col] = row[i]; });
        return obj;
      }
      return row;
    });
  }

  type StatsTab = 'ai' | 'tools' | 'network' | 'files' | 'snapshots';
  let activeTab = $state<StatsTab>('ai');

  let modelStats = $state<ModelStats[]>([]);
  let toolCalls = $state<ToolCallStat[]>([]);
  let networkEvents = $state<NetworkEvent[]>([]);
  let fileEvents = $state<FileEvent[]>([]);
  let loading = $state(false);

  // Snapshots
  interface SnapshotRow {
    id: number;
    timestamp: string;
    slot: number;
    origin: string;
    name: string | null;
    files_count: number;
    created: number;
    modified: number;
    deleted: number;
  }
  let snapshotStats = $state<{ total: number; auto_count: number; manual_count: number } | null>(null);
  let snapshotRows = $state<SnapshotRow[]>([]);

  onMount(async () => {
    if (!api.isConnected()) return;
    loading = true;
    try {
      const [aiResult, toolResult, netResult, fileResult] = await Promise.allSettled([
        api.inspectQuery(vmId, 'SELECT provider, model, SUM(input_tokens) as input_tokens, SUM(output_tokens) as output_tokens, SUM(estimated_cost_usd) as estimated_cost_usd, COUNT(*) as call_count FROM model_calls GROUP BY provider, model'),
        api.inspectQuery(vmId, 'SELECT tc.tool_name as tool, tc.origin as server, tc.arguments as args, tc.call_id, mc.timestamp FROM tool_calls tc JOIN model_calls mc ON tc.model_call_id = mc.id ORDER BY mc.timestamp DESC'),
        api.inspectQuery(vmId, 'SELECT method, domain || path as url, status_code as status, decision, duration_ms as durationMs, bytes_sent as bytesSent, bytes_received as bytesReceived, timestamp FROM net_events ORDER BY timestamp DESC'),
        api.inspectQuery(vmId, 'SELECT path, action as operation, size as sizeBytes, timestamp FROM fs_events ORDER BY timestamp DESC'),
      ]);
      if (aiResult.status === 'fulfilled' && aiResult.value.rows.length > 0) {
        modelStats = toObjects(aiResult.value).map((r: any) => ({
          provider: String(r.provider ?? ''),
          model: String(r.model ?? ''),
          inputTokens: Number(r.input_tokens ?? 0),
          outputTokens: Number(r.output_tokens ?? 0),
          cacheTokens: 0,
          estimatedCostUsd: Number(r.estimated_cost_usd ?? 0),
          callCount: Number(r.call_count ?? 0),
        }));
      }
      if (toolResult.status === 'fulfilled' && toolResult.value.rows.length > 0) {
        toolCalls = toObjects(toolResult.value).map((r: any, i: number) => ({
          id: `tc-${i}`, tool: String(r.tool ?? ''), server: String(r.server ?? ''),
          args: String(r.args ?? ''), result: '',
          durationMs: 0, timestamp: String(r.timestamp ?? ''),
        }));
      }
      if (netResult.status === 'fulfilled' && netResult.value.rows.length > 0) {
        networkEvents = toObjects(netResult.value).map((r: any, i: number) => ({
          id: `ne-${i}`, method: String(r.method ?? ''), url: String(r.url ?? ''),
          status: Number(r.status ?? 0), decision: r.decision === 'denied' ? 'denied' : 'allowed',
          durationMs: Number(r.durationMs ?? 0), bytesSent: Number(r.bytesSent ?? 0),
          bytesReceived: Number(r.bytesReceived ?? 0), timestamp: String(r.timestamp ?? ''),
        }));
      }
      if (fileResult.status === 'fulfilled' && fileResult.value.rows.length > 0) {
        fileEvents = toObjects(fileResult.value).map((r: any, i: number) => ({
          id: `fe-${i}`, path: String(r.path ?? ''), operation: r.operation as any,
          sizeBytes: r.sizeBytes != null ? Number(r.sizeBytes) : null,
          timestamp: String(r.timestamp ?? ''),
        }));
      }

      // Load snapshots
      const [snapStatsResult, snapListResult] = await Promise.allSettled([
        queryDb(SNAPSHOT_STATS_SQL),
        queryDb(SNAPSHOT_LIST_SQL),
      ]);
      if (snapStatsResult.status === 'fulfilled') {
        snapshotStats = queryOne(snapStatsResult.value);
      }
      if (snapListResult.status === 'fulfilled') {
        snapshotRows = queryAll(snapListResult.value);
      }
    } catch {
      // Keep empty state on error
    } finally {
      loading = false;
    }
  });

  // AI stats
  const totalInput = $derived(modelStats.reduce((s, m) => s + m.inputTokens, 0));
  const totalOutput = $derived(modelStats.reduce((s, m) => s + m.outputTokens, 0));
  const totalCost = $derived(modelStats.reduce((s, m) => s + m.estimatedCostUsd, 0));
  const totalCalls = $derived(modelStats.reduce((s, m) => s + m.callCount, 0));

  // Tools stats
  const toolTotal = $derived(toolCalls.length);
  const toolNative = $derived(toolCalls.filter(t => !t.server || t.server === 'system' || t.server === 'filesystem').length);
  const toolMcp = $derived(toolCalls.filter(t => t.server && t.server !== 'system' && t.server !== 'filesystem').length);

  // Network stats
  const netTotal = $derived(networkEvents.length);
  const netAllowed = $derived(networkEvents.filter(e => e.decision === 'allowed').length);
  const netDenied = $derived(networkEvents.filter(e => e.decision === 'denied').length);
  const netAvgLatency = $derived(
    networkEvents.length > 0
      ? Math.round(networkEvents.reduce((s, e) => s + e.durationMs, 0) / networkEvents.length)
      : 0,
  );

  // Files stats
  const fileTotal = $derived(fileEvents.length);
  const fileCreated = $derived(fileEvents.filter(e => e.operation === 'created').length);
  const fileModified = $derived(fileEvents.filter(e => e.operation === 'modified').length);
  const fileDeleted = $derived(fileEvents.filter(e => e.operation === 'deleted').length);

  const navItems: { id: StatsTab; label: string; icon: any }[] = [
    { id: 'ai', label: 'Model', icon: Brain },
    { id: 'tools', label: 'Tools', icon: Wrench },
    { id: 'network', label: 'Network', icon: Globe },
    { id: 'files', label: 'Files', icon: FileText },
    { id: 'snapshots', label: 'Snapshots', icon: ClockCounterClockwise },
  ];
</script>

<div class="flex h-full">
  <!-- Left nav -->
  <aside class="w-56 shrink-0 border-e border-line-2 bg-background overflow-y-auto py-4">
    <h1 class="text-xl font-bold text-foreground px-5 mb-4">Stats</h1>
    <nav class="space-y-0.5 px-3">
      {#each navItems as item (item.id)}
        <button
          type="button"
          class="w-full flex items-center gap-x-3 py-2 px-3 text-sm rounded-lg transition-colors
            {activeTab === item.id
              ? 'bg-muted text-foreground font-medium'
              : 'text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover'}"
          onclick={() => activeTab = item.id}
        >
          <item.icon size={18} />
          {item.label}
        </button>
      {/each}
    </nav>
  </aside>

  <!-- Content -->
  <main class="flex-1 overflow-y-auto">
    <div class="py-6 px-8">
    {#if activeTab === 'ai'}
      <h2 class="text-xl font-medium text-foreground mb-6">Model</h2>
      <div class="grid grid-cols-4 gap-3 mb-6">
        <div class="bg-card border border-card-line rounded-lg p-3">
          <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Total Calls</div>
          <div class="text-lg font-semibold text-foreground">{totalCalls}</div>
        </div>
        <div class="bg-card border border-card-line rounded-lg p-3">
          <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Input Tokens</div>
          <div class="text-lg font-semibold text-foreground">{totalInput.toLocaleString()}</div>
        </div>
        <div class="bg-card border border-card-line rounded-lg p-3">
          <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Output Tokens</div>
          <div class="text-lg font-semibold text-foreground">{totalOutput.toLocaleString()}</div>
        </div>
        <div class="bg-card border border-card-line rounded-lg p-3">
          <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Est. Cost</div>
          <div class="text-lg font-semibold text-foreground">${totalCost.toFixed(2)}</div>
        </div>
      </div>

      <!-- Per-model table -->
      <div class="bg-card border border-card-line rounded-xl overflow-hidden">
        <table class="w-full text-sm">
          <thead>
            <tr class="border-b border-card-divider bg-surface">
              <th class="text-left px-4 py-2 text-muted-foreground font-medium">Provider</th>
              <th class="text-left px-4 py-2 text-muted-foreground font-medium">Model</th>
              <th class="text-right px-4 py-2 text-muted-foreground font-medium">Calls</th>
              <th class="text-right px-4 py-2 text-muted-foreground font-medium">Input</th>
              <th class="text-right px-4 py-2 text-muted-foreground font-medium">Output</th>
              <th class="text-right px-4 py-2 text-muted-foreground font-medium">Cache</th>
              <th class="text-right px-4 py-2 text-muted-foreground font-medium">Cost</th>
            </tr>
          </thead>
          <tbody>
            {#each modelStats as model}
              <tr class="border-b border-card-divider last:border-0">
                <td class="px-4 py-2 text-foreground">{model.provider}</td>
                <td class="px-4 py-2 font-mono text-xs text-muted-foreground-1">{model.model}</td>
                <td class="px-4 py-2 text-right text-foreground">{model.callCount}</td>
                <td class="px-4 py-2 text-right text-foreground">{model.inputTokens.toLocaleString()}</td>
                <td class="px-4 py-2 text-right text-foreground">{model.outputTokens.toLocaleString()}</td>
                <td class="px-4 py-2 text-right text-muted-foreground-1">{model.cacheTokens.toLocaleString()}</td>
                <td class="px-4 py-2 text-right text-foreground">${model.estimatedCostUsd.toFixed(2)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>

    {:else if activeTab === 'tools'}
      <h2 class="text-xl font-medium text-foreground mb-6">Tools</h2>
      <div class="grid grid-cols-4 gap-3 mb-6">
        <div class="bg-card border border-card-line rounded-lg p-3">
          <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Total</div>
          <div class="text-lg font-semibold text-foreground">{toolTotal}</div>
        </div>
        <div class="bg-card border border-card-line rounded-lg p-3">
          <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Native</div>
          <div class="text-lg font-semibold text-foreground">{toolNative}</div>
        </div>
        <div class="bg-card border border-card-line rounded-lg p-3">
          <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">MCP</div>
          <div class="text-lg font-semibold text-foreground">{toolMcp}</div>
        </div>
        <div class="bg-card border border-card-line rounded-lg p-3">
          <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Calls</div>
          <div class="text-lg font-semibold text-foreground">{toolTotal}</div>
        </div>
      </div>
      <div class="bg-card border border-card-line rounded-xl overflow-hidden">
        <table class="w-full text-sm">
          <thead>
            <tr class="border-b border-card-divider bg-surface">
              <th class="text-left px-4 py-2 text-muted-foreground font-medium">Tool</th>
              <th class="text-left px-4 py-2 text-muted-foreground font-medium">Server</th>
              <th class="text-left px-4 py-2 text-muted-foreground font-medium">Arguments</th>
              <th class="text-left px-4 py-2 text-muted-foreground font-medium">Result</th>
              <th class="text-right px-4 py-2 text-muted-foreground font-medium">Duration</th>
              <th class="text-right px-4 py-2 text-muted-foreground font-medium">Time</th>
            </tr>
          </thead>
          <tbody>
            {#each toolCalls as call}
              <tr class="border-b border-card-divider last:border-0">
                <td class="px-4 py-2 font-mono text-xs text-foreground">{call.tool}</td>
                <td class="px-4 py-2 text-muted-foreground-1">{call.server}</td>
                <td class="px-4 py-2 font-mono text-xs text-muted-foreground-1 max-w-48 truncate">{truncate(call.args, 40)}</td>
                <td class="px-4 py-2 font-mono text-xs text-muted-foreground-1 max-w-48 truncate">{truncate(call.result, 40)}</td>
                <td class="px-4 py-2 text-right text-foreground">{formatDuration(call.durationMs)}</td>
                <td class="px-4 py-2 text-right text-muted-foreground">{formatTime(call.timestamp)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>

    {:else if activeTab === 'network'}
      <h2 class="text-xl font-medium text-foreground mb-6">Network</h2>
      <div class="grid grid-cols-4 gap-3 mb-6">
        <div class="bg-card border border-card-line rounded-lg p-3">
          <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Total</div>
          <div class="text-lg font-semibold text-foreground">{netTotal}</div>
        </div>
        <div class="bg-card border border-card-line rounded-lg p-3">
          <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Allowed</div>
          <div class="text-lg font-semibold text-primary">{netAllowed}</div>
        </div>
        <div class="bg-card border border-card-line rounded-lg p-3">
          <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Denied</div>
          <div class="text-lg font-semibold text-destructive">{netDenied}</div>
        </div>
        <div class="bg-card border border-card-line rounded-lg p-3">
          <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Avg Latency</div>
          <div class="text-lg font-semibold text-foreground">{netAvgLatency}ms</div>
        </div>
      </div>
      <div class="bg-card border border-card-line rounded-xl overflow-hidden">
        <table class="w-full text-sm">
          <thead>
            <tr class="border-b border-card-divider bg-surface">
              <th class="text-left px-4 py-2 text-muted-foreground font-medium">Method</th>
              <th class="text-left px-4 py-2 text-muted-foreground font-medium">URL</th>
              <th class="text-center px-4 py-2 text-muted-foreground font-medium">Status</th>
              <th class="text-center px-4 py-2 text-muted-foreground font-medium">Decision</th>
              <th class="text-right px-4 py-2 text-muted-foreground font-medium">Duration</th>
              <th class="text-right px-4 py-2 text-muted-foreground font-medium">Size</th>
              <th class="text-right px-4 py-2 text-muted-foreground font-medium">Time</th>
            </tr>
          </thead>
          <tbody>
            {#each networkEvents as event}
              <tr class="border-b border-card-divider last:border-0">
                <td class="px-4 py-2 font-mono text-xs font-semibold text-foreground">{event.method}</td>
                <td class="px-4 py-2 font-mono text-xs text-muted-foreground-1 max-w-64 truncate">{event.url}</td>
                <td class="px-4 py-2 text-center">
                  {#if event.status > 0}
                    <span class="font-mono text-xs {event.status < 400 ? 'text-primary' : 'text-destructive'}">{event.status}</span>
                  {:else}
                    <span class="text-xs text-muted-foreground">--</span>
                  {/if}
                </td>
                <td class="px-4 py-2 text-center">
                  <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium {event.decision === 'allowed'
                    ? 'bg-primary/10 text-primary'
                    : 'bg-destructive/10 text-destructive'}">
                    {event.decision}
                  </span>
                </td>
                <td class="px-4 py-2 text-right text-foreground">{event.durationMs > 0 ? formatDuration(event.durationMs) : '--'}</td>
                <td class="px-4 py-2 text-right text-muted-foreground">{event.bytesReceived > 0 ? formatBytes(event.bytesReceived) : '--'}</td>
                <td class="px-4 py-2 text-right text-muted-foreground">{formatTime(event.timestamp)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>

    {:else if activeTab === 'files'}
      <h2 class="text-xl font-medium text-foreground mb-6">Files</h2>
      <div class="grid grid-cols-4 gap-3 mb-6">
        <div class="bg-card border border-card-line rounded-lg p-3">
          <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Total</div>
          <div class="text-lg font-semibold text-foreground">{fileTotal}</div>
        </div>
        <div class="bg-card border border-card-line rounded-lg p-3">
          <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Created</div>
          <div class="text-lg font-semibold text-primary">{fileCreated}</div>
        </div>
        <div class="bg-card border border-card-line rounded-lg p-3">
          <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Modified</div>
          <div class="text-lg font-semibold text-foreground">{fileModified}</div>
        </div>
        <div class="bg-card border border-card-line rounded-lg p-3">
          <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Deleted</div>
          <div class="text-lg font-semibold text-destructive">{fileDeleted}</div>
        </div>
      </div>
      <div class="bg-card border border-card-line rounded-xl overflow-hidden">
        <table class="w-full text-sm">
          <thead>
            <tr class="border-b border-card-divider bg-surface">
              <th class="text-left px-4 py-2 text-muted-foreground font-medium">Path</th>
              <th class="text-center px-4 py-2 text-muted-foreground font-medium">Operation</th>
              <th class="text-right px-4 py-2 text-muted-foreground font-medium">Size</th>
              <th class="text-right px-4 py-2 text-muted-foreground font-medium">Time</th>
            </tr>
          </thead>
          <tbody>
            {#each fileEvents as event}
              <tr class="border-b border-card-divider last:border-0">
                <td class="px-4 py-2 font-mono text-xs text-foreground">{event.path}</td>
                <td class="px-4 py-2 text-center">
                  <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium
                    {event.operation === 'created' ? 'bg-primary/10 text-primary' :
                     event.operation === 'deleted' ? 'bg-destructive/10 text-destructive' :
                     'bg-muted text-muted-foreground-1'}">
                    {event.operation}
                  </span>
                </td>
                <td class="px-4 py-2 text-right text-muted-foreground">{event.sizeBytes != null ? formatBytes(event.sizeBytes) : '--'}</td>
                <td class="px-4 py-2 text-right text-muted-foreground">{formatTime(event.timestamp)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>

    {:else if activeTab === 'snapshots'}
      <h2 class="text-xl font-medium text-foreground mb-6">Snapshots</h2>
      {#if snapshotStats}
        <div class="grid grid-cols-3 gap-3 mb-6">
          <div class="bg-card border border-card-line rounded-lg p-3">
            <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Total</div>
            <div class="text-lg font-semibold text-foreground">{snapshotStats.total}</div>
          </div>
          <div class="bg-card border border-card-line rounded-lg p-3">
            <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Auto</div>
            <div class="text-lg font-semibold text-foreground">{snapshotStats.auto_count}</div>
          </div>
          <div class="bg-card border border-card-line rounded-lg p-3">
            <div class="text-[11px] text-muted-foreground mb-0.5 uppercase tracking-wider">Manual</div>
            <div class="text-lg font-semibold text-foreground">{snapshotStats.manual_count}</div>
          </div>
        </div>
      {/if}

      {#if snapshotRows.length === 0}
        <div class="flex items-center justify-center h-32 text-sm text-muted-foreground">
          No snapshots yet.
        </div>
      {:else}
        <div class="bg-card border border-card-line rounded-xl overflow-hidden">
          <table class="w-full text-sm">
            <thead>
              <tr class="border-b border-card-divider bg-surface">
                <th class="text-left px-4 py-2 text-muted-foreground font-medium w-20">Slot</th>
                <th class="text-left px-4 py-2 text-muted-foreground font-medium">Name</th>
                <th class="text-left px-4 py-2 text-muted-foreground font-medium w-24">Age</th>
                <th class="text-right px-4 py-2 text-muted-foreground font-medium w-16">Files</th>
                <th class="text-right px-4 py-2 text-muted-foreground font-medium w-16">Added</th>
                <th class="text-right px-4 py-2 text-muted-foreground font-medium w-16">Modified</th>
                <th class="text-right px-4 py-2 text-muted-foreground font-medium w-16">Deleted</th>
              </tr>
            </thead>
            <tbody>
              {#each snapshotRows as snap}
                <tr class="border-b border-card-divider last:border-0 hover:bg-muted-hover">
                  <td class="px-4 py-2">
                    <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium
                      {snap.origin === 'manual' ? 'bg-primary/10 text-primary' : 'bg-muted text-muted-foreground-1'}">
                      cp-{snap.slot}
                    </span>
                  </td>
                  <td class="px-4 py-2 font-medium text-foreground">{snap.name ?? ''}</td>
                  <td class="px-4 py-2 text-xs text-muted-foreground">{fmtAge(snap.timestamp)}</td>
                  <td class="px-4 py-2 text-right tabular-nums text-muted-foreground">{snap.files_count || ''}</td>
                  <td class="px-4 py-2 text-right tabular-nums">
                    {#if snap.created > 0}<span class="text-primary">{snap.created}</span>{/if}
                  </td>
                  <td class="px-4 py-2 text-right tabular-nums">
                    {#if snap.modified > 0}<span class="text-foreground">{snap.modified}</span>{/if}
                  </td>
                  <td class="px-4 py-2 text-right tabular-nums">
                    {#if snap.deleted > 0}<span class="text-destructive">{snap.deleted}</span>{/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    {/if}
    </div>
  </main>
</div>
