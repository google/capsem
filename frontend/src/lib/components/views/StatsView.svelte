<script lang="ts">
  import { onMount } from 'svelte';
  import * as api from '../../api';
  import { SNAPSHOT_STATS_SQL, SNAPSHOT_LIST_SQL } from '../../sql';
  import type { ModelStats, ToolCallStat, NetworkEvent, FileEvent, DetailSelection } from '../../types';
  import { formatDuration, formatBytes, formatTime, truncate, fmtAge } from '../../format';
  import { getShikiHighlighter, resolveShikiTheme, type ShikiHighlighter } from '../../shiki.ts';
  import { themeStore } from '../../stores/theme.svelte.ts';
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

  // Detail panel
  let detail = $state<DetailSelection | null>(null);
  let shiki = $state<ShikiHighlighter | null>(null);


  // Shiki is loaded in the main onMount below
  function shikiHighlight(text: string, lang: string): string {
    if (!shiki) return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
    const theme = resolveShikiTheme(themeStore.terminalTheme, themeStore.mode);
    return shiki.codeToHtml(text, { lang, theme });
  }

  function formatAndHighlight(text: string | null | undefined, lang?: string): string {
    if (!text) return '';
    const trimmed = text.trim();
    if (!trimmed) return '';
    // Auto-detect JSON
    const isJson = trimmed.startsWith('{') || trimmed.startsWith('[');
    const detectedLang = lang ?? (isJson ? 'json' : 'text');
    let content = trimmed;
    if (isJson) {
      try { content = JSON.stringify(JSON.parse(trimmed), null, 2); } catch { /* keep original */ }
    }
    return shikiHighlight(content, detectedLang);
  }

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
    getShikiHighlighter().then(h => { shiki = h; });
    if (!api.isConnected()) return;
    loading = true;
    try {
      const [aiResult, toolResult, netResult, fileResult] = await Promise.allSettled([
        api.inspectQuery(vmId, 'SELECT provider, model, SUM(input_tokens) as input_tokens, SUM(output_tokens) as output_tokens, SUM(estimated_cost_usd) as estimated_cost_usd, COUNT(*) as call_count FROM model_calls GROUP BY provider, model'),
        api.inspectQuery(vmId, 'SELECT tc.tool_name as tool, tc.origin as server, tc.arguments as args, tc.call_id, mc.timestamp, tr.content_preview as result, tr.is_error FROM tool_calls tc JOIN model_calls mc ON tc.model_call_id = mc.id LEFT JOIN tool_responses tr ON tc.call_id = tr.call_id ORDER BY mc.timestamp DESC'),
        api.inspectQuery(vmId, 'SELECT method, domain, path, domain || path as url, status_code as status, decision, duration_ms as durationMs, bytes_sent as bytesSent, bytes_received as bytesReceived, timestamp, request_headers, response_headers, request_body_preview, response_body_preview, matched_rule FROM net_events ORDER BY timestamp DESC'),
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
          args: String(r.args ?? ''), result: String(r.result ?? ''),
          durationMs: 0, timestamp: String(r.timestamp ?? ''),
          isError: Number(r.is_error ?? 0),
        }));
      }
      if (netResult.status === 'fulfilled' && netResult.value.rows.length > 0) {
        networkEvents = toObjects(netResult.value).map((r: any, i: number) => ({
          id: `ne-${i}`, method: String(r.method ?? ''), url: String(r.url ?? ''),
          domain: String(r.domain ?? ''), path: String(r.path ?? '/'),
          status: Number(r.status ?? 0), decision: r.decision === 'denied' ? 'denied' : 'allowed',
          durationMs: Number(r.durationMs ?? 0), bytesSent: Number(r.bytesSent ?? 0),
          bytesReceived: Number(r.bytesReceived ?? 0), timestamp: String(r.timestamp ?? ''),
          requestHeaders: r.request_headers as string | null,
          responseHeaders: r.response_headers as string | null,
          requestBodyPreview: r.request_body_preview as string | null,
          responseBodyPreview: r.response_body_preview as string | null,
          matchedRule: r.matched_rule as string | null,
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
        api.inspectQuery(vmId, SNAPSHOT_STATS_SQL),
        api.inspectQuery(vmId, SNAPSHOT_LIST_SQL),
      ]);
      if (snapStatsResult.status === 'fulfilled' && snapStatsResult.value.rows.length > 0) {
        const row = toObjects(snapStatsResult.value)[0];
        snapshotStats = {
          total: Number(row.total ?? 0),
          auto_count: Number(row.auto_count ?? 0),
          manual_count: Number(row.manual_count ?? 0),
        };
      }
      if (snapListResult.status === 'fulfilled' && snapListResult.value.rows.length > 0) {
        snapshotRows = toObjects(snapListResult.value).map((r: any) => ({
          id: Number(r.id),
          timestamp: String(r.timestamp ?? ''),
          slot: Number(r.slot ?? 0),
          origin: String(r.origin ?? ''),
          name: r.name != null ? String(r.name) : null,
          files_count: Number(r.files_count ?? 0),
          created: Number(r.created ?? 0),
          modified: Number(r.modified ?? 0),
          deleted: Number(r.deleted ?? 0),
        }));
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
          onclick={() => { activeTab = item.id; detail = null; }}
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
              <tr class="border-b border-card-divider last:border-0 hover:bg-muted-hover cursor-pointer"
                  onclick={() => detail = { type: 'tool', data: { tool_name: call.tool, origin: call.server, arguments: call.args, content_preview: call.result, is_error: call.isError, timestamp: call.timestamp } }}>
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
              <tr class="border-b border-card-divider last:border-0 hover:bg-muted-hover cursor-pointer"
                  onclick={() => detail = { type: 'net_event', data: { method: event.method, domain: event.domain, path: event.path, decision: event.decision, status_code: event.status, duration_ms: event.durationMs, matched_rule: event.matchedRule, request_headers: event.requestHeaders, request_body_preview: event.requestBodyPreview, response_headers: event.responseHeaders, response_body_preview: event.responseBodyPreview, timestamp: event.timestamp } }}>
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
              <tr class="border-b border-card-divider last:border-0 hover:bg-muted-hover cursor-pointer"
                  onclick={() => detail = { type: 'file_event', data: { action: event.operation, path: event.path, size: event.sizeBytes, timestamp: event.timestamp } }}>
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

  <!-- Detail panel (slide from right) -->
  {#if detail}
    {@const d = detail.data}
    <div class="w-[400px] shrink-0 border-s border-line-2 flex flex-col overflow-hidden bg-background">
      <!-- Header -->
      <div class="flex items-center gap-2 px-3 py-2 border-b border-line-2 bg-surface">
        <span class="text-xs font-semibold flex-1 truncate capitalize text-foreground">{detail.type.replace('_', ' ')}</span>
        <button class="p-1 rounded hover:bg-muted-hover text-muted-foreground-1 hover:text-foreground" onclick={() => detail = null} aria-label="Close detail panel">
          <svg class="size-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
        </button>
      </div>

      <!-- Content -->
      <div class="flex-1 overflow-auto p-3 text-xs space-y-3">
        {#if detail.type === 'tool'}
          <div>
            <div class="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider mb-1">
              {d.tool_name ?? 'Tool'}
              {#if d.origin && d.origin !== 'native'}
                <span class="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] bg-muted text-muted-foreground-1 ml-1">{d.origin}</span>
              {/if}
            </div>
          </div>
          <div>
            <div class="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider mb-1">Arguments</div>
            <div class="detail-shiki rounded overflow-auto max-h-64 bg-background-1">{@html formatAndHighlight(d.arguments as string, 'json')}</div>
          </div>
          {#if d.content_preview !== undefined}
            <div>
              <div class="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider mb-1">
                Result
                {#if d.is_error}
                  <span class="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] bg-destructive/10 text-destructive ml-1">error</span>
                {/if}
              </div>
              {#if d.content_preview}
                <div class="detail-shiki rounded overflow-auto max-h-64 bg-background-1">{@html formatAndHighlight(d.content_preview as string, 'json')}</div>
              {:else}
                <div class="text-muted-foreground italic px-2 py-1">(empty)</div>
              {/if}
            </div>
          {/if}

        {:else if detail.type === 'net_event'}
          <div class="space-y-1">
            <div><span class="text-muted-foreground">Method:</span> <span class="font-mono text-foreground">{d.method ?? 'CONNECT'}</span></div>
            <div><span class="text-muted-foreground">Domain:</span> <span class="font-mono text-foreground">{d.domain}</span></div>
            <div><span class="text-muted-foreground">Path:</span> <span class="font-mono text-foreground">{d.path ?? '/'}</span></div>
            <div>
              <span class="text-muted-foreground">Decision:</span>
              <span class="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium {d.decision === 'allowed' ? 'bg-primary/10 text-primary' : 'bg-destructive/10 text-destructive'}">{d.decision}</span>
            </div>
            {#if d.status_code}
              <div><span class="text-muted-foreground">Status:</span> <span class="font-mono text-foreground">{d.status_code}</span></div>
            {/if}
            {#if d.duration_ms}
              <div><span class="text-muted-foreground">Duration:</span> <span class="font-mono text-foreground">{d.duration_ms}ms</span></div>
            {/if}
            {#if d.matched_rule}
              <div><span class="text-muted-foreground">Rule:</span> <span class="font-mono text-foreground">{d.matched_rule}</span></div>
            {/if}
          </div>
          {#if d.request_headers}
            <div>
              <div class="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider mb-1">Request Headers</div>
              <div class="detail-shiki rounded overflow-auto max-h-40 bg-background-1">{@html formatAndHighlight(d.request_headers as string, 'bash')}</div>
            </div>
          {/if}
          {#if d.request_body_preview}
            <div>
              <div class="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider mb-1">Request Body</div>
              <div class="detail-shiki rounded overflow-auto max-h-40 bg-background-1">{@html formatAndHighlight(d.request_body_preview as string, 'json')}</div>
            </div>
          {/if}
          {#if d.response_headers}
            <div>
              <div class="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider mb-1">Response Headers</div>
              <div class="detail-shiki rounded overflow-auto max-h-40 bg-background-1">{@html formatAndHighlight(d.response_headers as string, 'bash')}</div>
            </div>
          {/if}
          {#if d.response_body_preview}
            <div>
              <div class="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider mb-1">Response Body</div>
              <div class="detail-shiki rounded overflow-auto max-h-40 bg-background-1">{@html formatAndHighlight(d.response_body_preview as string, 'json')}</div>
            </div>
          {/if}

        {:else if detail.type === 'file_event'}
          <div class="space-y-1">
            <div>
              <span class="text-muted-foreground">Action:</span>
              <span class="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium
                {d.action === 'deleted' ? 'bg-destructive/10 text-destructive' : d.action === 'created' ? 'bg-primary/10 text-primary' : 'bg-muted text-muted-foreground-1'}">{d.action}</span>
            </div>
            <div><span class="text-muted-foreground">Path:</span> <span class="font-mono text-foreground break-all">{d.path}</span></div>
            {#if d.size != null}
              <div><span class="text-muted-foreground">Size:</span> <span class="font-mono text-foreground">{formatBytes(d.size as number)}</span></div>
            {/if}
            {#if d.timestamp}
              <div><span class="text-muted-foreground">Time:</span> <span class="font-mono text-foreground">{d.timestamp}</span></div>
            {/if}
          </div>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .detail-shiki :global(pre.shiki) {
    margin: 0;
    padding: 0.5rem 0.75rem;
    background: transparent !important;
    font-size: 0.75rem;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
