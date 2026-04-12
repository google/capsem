<script lang="ts">
  import { onMount } from 'svelte';
  import * as api from '../../api';
  import { mockModelStats, mockToolCalls, mockNetworkEvents, mockFileEvents } from '../../mock.ts';
  import type { MockModelStats, MockToolCall, MockNetworkEvent, MockFileEvent } from '../../mock.ts';
  import type { InspectResponse } from '../../types/gateway';

  let { vmId }: { vmId: string } = $props();

  type StatsTab = 'ai' | 'tools' | 'network' | 'files';
  let activeTab = $state<StatsTab>('ai');

  // Live data (falls back to mock)
  let modelStats = $state<MockModelStats[]>(mockModelStats);
  let toolCalls = $state<MockToolCall[]>(mockToolCalls);
  let networkEvents = $state<MockNetworkEvent[]>(mockNetworkEvents);
  let fileEvents = $state<MockFileEvent[]>(mockFileEvents);
  let loading = $state(false);

  onMount(async () => {
    if (!api.isConnected()) return;
    loading = true;
    try {
      const [aiResult, toolResult, netResult, fileResult] = await Promise.allSettled([
        api.inspectQuery(vmId, 'SELECT provider, model, input_tokens, output_tokens, cache_tokens, estimated_cost_usd, call_count FROM model_calls'),
        api.inspectQuery(vmId, 'SELECT tool_name as tool, server, args, result, duration_ms as durationMs, timestamp FROM tool_calls ORDER BY timestamp DESC'),
        api.inspectQuery(vmId, 'SELECT method, url, status_code as status, decision, duration_ms as durationMs, bytes_sent as bytesSent, bytes_received as bytesReceived, timestamp FROM http_requests ORDER BY timestamp DESC'),
        api.inspectQuery(vmId, 'SELECT path, operation, size_bytes as sizeBytes, timestamp FROM file_events ORDER BY timestamp DESC'),
      ]);
      if (aiResult.status === 'fulfilled' && aiResult.value.rows.length > 0) {
        modelStats = aiResult.value.rows.map((r: any) => ({
          provider: String(r.provider ?? ''),
          model: String(r.model ?? ''),
          inputTokens: Number(r.input_tokens ?? 0),
          outputTokens: Number(r.output_tokens ?? 0),
          cacheTokens: Number(r.cache_tokens ?? 0),
          estimatedCostUsd: Number(r.estimated_cost_usd ?? 0),
          callCount: Number(r.call_count ?? 0),
        }));
      }
      if (toolResult.status === 'fulfilled' && toolResult.value.rows.length > 0) {
        toolCalls = toolResult.value.rows.map((r: any, i: number) => ({
          id: `tc-${i}`, tool: String(r.tool ?? ''), server: String(r.server ?? ''),
          args: String(r.args ?? ''), result: String(r.result ?? ''),
          durationMs: Number(r.durationMs ?? 0), timestamp: String(r.timestamp ?? ''),
        }));
      }
      if (netResult.status === 'fulfilled' && netResult.value.rows.length > 0) {
        networkEvents = netResult.value.rows.map((r: any, i: number) => ({
          id: `ne-${i}`, method: String(r.method ?? ''), url: String(r.url ?? ''),
          status: Number(r.status ?? 0), decision: r.decision === 'denied' ? 'denied' : 'allowed',
          durationMs: Number(r.durationMs ?? 0), bytesSent: Number(r.bytesSent ?? 0),
          bytesReceived: Number(r.bytesReceived ?? 0), timestamp: String(r.timestamp ?? ''),
        }));
      }
      if (fileResult.status === 'fulfilled' && fileResult.value.rows.length > 0) {
        fileEvents = fileResult.value.rows.map((r: any, i: number) => ({
          id: `fe-${i}`, path: String(r.path ?? ''), operation: r.operation as any,
          sizeBytes: r.sizeBytes != null ? Number(r.sizeBytes) : null,
          timestamp: String(r.timestamp ?? ''),
        }));
      }
    } catch {
      // Keep mock data on error
    } finally {
      loading = false;
    }
  });

  const totalInput = $derived(modelStats.reduce((s, m) => s + m.inputTokens, 0));
  const totalOutput = $derived(modelStats.reduce((s, m) => s + m.outputTokens, 0));
  const totalCost = $derived(modelStats.reduce((s, m) => s + m.estimatedCostUsd, 0));
  const totalCalls = $derived(modelStats.reduce((s, m) => s + m.callCount, 0));

  function formatDuration(ms: number): string {
    if (ms < 1000) return `${ms}ms`;
    return `${(ms / 1000).toFixed(1)}s`;
  }

  function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  function formatTime(iso: string): string {
    return new Date(iso).toLocaleTimeString();
  }

  function truncate(s: string, max: number): string {
    return s.length > max ? s.slice(0, max) + '...' : s;
  }

  const tabs: { id: StatsTab; label: string }[] = [
    { id: 'ai', label: 'AI' },
    { id: 'tools', label: 'Tools' },
    { id: 'network', label: 'Network' },
    { id: 'files', label: 'Files' },
  ];
</script>

<div class="flex flex-col h-full">
  <!-- Tab bar -->
  <div class="flex items-center gap-x-1 border-b border-line-2 bg-layer px-4 py-1.5">
    {#each tabs as tab}
      <button
        type="button"
        class="px-3 py-1 text-sm rounded-lg transition-colors {activeTab === tab.id
          ? 'bg-primary text-primary-foreground'
          : 'text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover'}"
        onclick={() => activeTab = tab.id}
      >
        {tab.label}
      </button>
    {/each}
  </div>

  <!-- Tab content -->
  <div class="flex-1 overflow-auto p-4">
    {#if activeTab === 'ai'}
      <!-- Summary cards -->
      <div class="grid grid-cols-4 gap-3 mb-6">
        <div class="bg-card border border-card-line rounded-xl p-4">
          <div class="text-xs text-muted-foreground mb-1">Total Calls</div>
          <div class="text-2xl font-semibold text-foreground">{totalCalls}</div>
        </div>
        <div class="bg-card border border-card-line rounded-xl p-4">
          <div class="text-xs text-muted-foreground mb-1">Input Tokens</div>
          <div class="text-2xl font-semibold text-foreground">{totalInput.toLocaleString()}</div>
        </div>
        <div class="bg-card border border-card-line rounded-xl p-4">
          <div class="text-xs text-muted-foreground mb-1">Output Tokens</div>
          <div class="text-2xl font-semibold text-foreground">{totalOutput.toLocaleString()}</div>
        </div>
        <div class="bg-card border border-card-line rounded-xl p-4">
          <div class="text-xs text-muted-foreground mb-1">Est. Cost</div>
          <div class="text-2xl font-semibold text-foreground">${totalCost.toFixed(2)}</div>
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
    {/if}
  </div>
</div>
