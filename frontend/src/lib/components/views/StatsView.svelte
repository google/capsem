<script lang="ts">
  import { onMount } from 'svelte';
  import * as api from '../../api';
  import type { InspectResponse } from '../../types/gateway';
  import { formatBytes, formatDuration, formatTime } from '../../format';
  import { getShikiHighlighter, resolveShikiTheme, ensureShikiLang, ensureShikiTheme, type ShikiHighlighter } from '../../shiki.ts';
  import { themeStore } from '../../stores/theme.svelte.ts';
  import { tabStore } from '../../stores/tabs.svelte.ts';
  import MetricCard from './stats/MetricCard.svelte';
  import StatsBadge from './stats/StatsBadge.svelte';
  import StatsEventList from './stats/StatsEventList.svelte';
  import StatsMiniGroup from './stats/StatsMiniGroup.svelte';
  import StatsTable from './stats/StatsTable.svelte';
  import Brain from 'phosphor-svelte/lib/Brain';
  import Wrench from 'phosphor-svelte/lib/Wrench';
  import Globe from 'phosphor-svelte/lib/Globe';
  import FileText from 'phosphor-svelte/lib/FileText';
  import ShieldCheck from 'phosphor-svelte/lib/ShieldCheck';
  import Database from 'phosphor-svelte/lib/Database';
  import Terminal from 'phosphor-svelte/lib/Terminal';
  import DotsThreeCircle from 'phosphor-svelte/lib/DotsThreeCircle';
  import Fingerprint from 'phosphor-svelte/lib/Fingerprint';

  let { vmId }: { vmId: string } = $props();

  type StatsTab = 'model' | 'mcp' | 'http' | 'dns' | 'files' | 'process' | 'credentials' | 'security';
  type DetailSelection = { type: string; data: Record<string, unknown> };
  type Row = Record<string, any>;
  const SECURITY_ACTIONS: api.SecurityRuleAction[] = ['allow', 'ask', 'block', 'preprocess', 'rewrite', 'postprocess'];
  const SECURITY_DETECTION_LEVELS: api.RuntimeSecurityRuleDetectionLevel[] = ['none', 'informational', 'low', 'medium', 'high', 'critical'];

  let activeTab = $state<StatsTab>('model');
  let loading = $state(false);
  let error = $state<string | null>(null);
  let detail = $state<DetailSelection | null>(null);
  let shiki = $state<ShikiHighlighter | null>(null);
  let shikiTick = $state(0);

  let modelStats = $state<Row[]>([]);
  let modelRows = $state<Row[]>([]);
  let mcpRows = $state<Row[]>([]);
  let httpRows = $state<Row[]>([]);
  let dnsRows = $state<Row[]>([]);
  let fileRows = $state<Row[]>([]);
  let processRows = $state<Row[]>([]);
  let auditRows = $state<Row[]>([]);
  let substitutionRows = $state<Row[]>([]);
  let securityLatest = $state<api.SecurityRuleEvent[]>([]);
  let detectionLatest = $state<api.SecurityRuleEvent[]>([]);
  let enforcementLatest = $state<api.SecurityRuleEvent[]>([]);
  let securityStatus = $state<api.SecurityRuleStats | null>(null);

  function inspectTab() {
    const current = tabStore.active;
    if (current?.vmId === vmId) {
      tabStore.updateView(current.id, 'inspector');
    } else {
      tabStore.add('inspector', 'Inspector', vmId);
    }
  }

  function toObjects(resp: InspectResponse): Row[] {
    return resp.rows.map((row: any) => {
      if (!Array.isArray(row)) return row;
      const obj: Row = {};
      resp.columns.forEach((col, index) => { obj[col] = row[index]; });
      return obj;
    });
  }

  async function query(sql: string): Promise<Row[]> {
    return toObjects(await api.inspectQuery(vmId, sql));
  }

  function number(value: unknown): number {
    const n = Number(value ?? 0);
    return Number.isFinite(n) ? n : 0;
  }

  function text(value: unknown): string {
    return value == null ? '' : String(value);
  }

  function eventTimeMs(value: number): string {
    return new Date(value).toISOString();
  }

  function entries(obj: Record<string, unknown>): [string, unknown][] {
    return Object.entries(obj);
  }

  const DETAIL_PAYLOAD_KEYS = new Set([
    'request_headers',
    'response_headers',
    'request_body_preview',
    'response_body_preview',
    'request_preview',
    'response_preview',
    'text_content',
    'context_json',
  ]);

  const DETAIL_STRUCTURED_KEYS = new Set([
    'rule_json',
    'event_json',
  ]);

  const DETAIL_HIDDEN_KEYS = new Set([
    'substitution_ref',
    'credential_ref',
  ]);

  function isPresent(value: unknown): boolean {
    if (value == null) return false;
    if (typeof value === 'string') return value.trim().length > 0;
    return true;
  }

  function labelForDetailKey(key: string): string {
    return key
      .split('_')
      .map(part => part.charAt(0).toUpperCase() + part.slice(1))
      .join(' ');
  }

  function visibleDetailEntries(obj: Record<string, unknown>): [string, unknown][] {
    return entries(obj).filter(([key, value]) => (
      isPresent(value)
      && !DETAIL_PAYLOAD_KEYS.has(key)
      && !DETAIL_STRUCTURED_KEYS.has(key)
      && !DETAIL_HIDDEN_KEYS.has(key)
    ));
  }

  function detailPayloadSections(obj: Record<string, unknown>): { key: string; label: string; value: unknown; lang: string }[] {
    return entries(obj)
      .filter(([key, value]) => DETAIL_PAYLOAD_KEYS.has(key) && isPresent(value))
      .map(([key, value]) => ({
        key,
        label: labelForDetailKey(key),
        value,
        lang: detailPayloadLang(key, value),
      }));
  }

  function detailPayloadLang(key: string, value: unknown): string {
    if (key.endsWith('_headers')) return 'http';
    if (key === 'context_json') return 'json';
    const content = normalizePreviewContent(typeof value === 'string' ? value : JSON.stringify(value));
    const trimmed = content.trim();
    if (trimmed.startsWith('{') || trimmed.startsWith('[')) {
      try {
        JSON.parse(trimmed);
        return 'json';
      } catch {
        return 'text';
      }
    }
    return 'text';
  }

  function formatDetailValue(value: unknown): string {
    if (value == null) return 'NULL';
    if (typeof value === 'object') return JSON.stringify(value);
    return String(value);
  }

  function normalizePreviewContent(content: string): string {
    const trimmed = content.trim();
    if (!trimmed) return content;
    if (
      (trimmed.startsWith('{') || trimmed.startsWith('['))
      && (trimmed.includes('\\"') || trimmed.includes('\\n') || trimmed.includes('\\t'))
    ) {
      const unescaped = trimmed
        .replace(/\\n/g, '\n')
        .replace(/\\r/g, '\r')
        .replace(/\\t/g, '\t')
        .replace(/\\"/g, '"');
      try {
        JSON.parse(unescaped);
        return unescaped;
      } catch {
        return content;
      }
    }
    return content;
  }

  function formatAndHighlight(value: unknown, lang?: string): string {
    shikiTick;
    if (value == null) return '';
    let content = typeof value === 'string' ? value : JSON.stringify(value, null, 2);
    content = normalizePreviewContent(content);
    const trimmed = content.trim();
    if (!trimmed) return '';
    const isJson = trimmed.startsWith('{') || trimmed.startsWith('[');
    const detectedLang = lang ?? (isJson ? 'json' : 'text');
    if (isJson) {
      try { content = JSON.stringify(JSON.parse(trimmed), null, 2); } catch { content = trimmed; }
    }
    if (!shiki) return content.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
    const theme = resolveShikiTheme(themeStore.terminalTheme, themeStore.mode);
    if (!shiki.getLoadedLanguages().includes(detectedLang) || !shiki.getLoadedThemes().includes(theme)) {
      return content.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
    }
    return shiki.codeToHtml(content, { lang: detectedLang, theme });
  }

  $effect(() => {
    const theme = resolveShikiTheme(themeStore.terminalTheme, themeStore.mode);
    Promise.all([
      ensureShikiLang('json'),
      ensureShikiLang('http'),
      ensureShikiLang('sql'),
      ensureShikiTheme(theme),
    ]).then(() => { shikiTick++; }).catch(() => {});
  });

  async function load() {
    if (!api.isConnected()) return;
    loading = true;
    error = null;
    try {
      const [
        modelStatsRows,
        modelEventRows,
        mcpEventRows,
        httpEventRows,
        dnsEventRows,
        fsEventRows,
        processEventRows,
        auditEventRows,
        substitutionEventRows,
        secLatest,
        secStatus,
        detLatest,
        enfLatest,
      ] = await Promise.all([
        query(`SELECT provider, COALESCE(model, 'unknown') AS model,
                 COUNT(*) AS call_count,
                 COALESCE(SUM(input_tokens), 0) AS input_tokens,
                 COALESCE(SUM(output_tokens), 0) AS output_tokens,
                 COALESCE(SUM(estimated_cost_usd), 0.0) AS estimated_cost_usd,
                 COALESCE(SUM(duration_ms), 0) AS duration_ms
               FROM model_calls
               GROUP BY provider, model
               ORDER BY call_count DESC, provider ASC`),
        query(`SELECT event_id, timestamp, provider, model, method, path, status_code,
                 input_tokens, output_tokens, duration_ms, response_bytes,
                 stop_reason, trace_id, credential_ref, request_body_preview, text_content
               FROM model_calls
               ORDER BY id DESC
               LIMIT 200`),
        query(`SELECT event_id, timestamp, server_name, method, tool_name, request_id,
                 decision, duration_ms, bytes_sent, bytes_received, policy_rule,
                 trace_id, credential_ref, request_preview, response_preview, error_message
               FROM mcp_calls
               ORDER BY id DESC
               LIMIT 200`),
        query(`SELECT event_id, timestamp, domain, port, method, path, query, status_code,
                 decision, duration_ms, bytes_sent, bytes_received, matched_rule, policy_rule,
                 trace_id, credential_ref, request_headers, response_headers,
                 request_body_preview, response_body_preview
               FROM net_events
               ORDER BY id DESC
               LIMIT 200`),
        query(`SELECT event_id, timestamp, qname, qtype, qclass, rcode, decision,
                 matched_rule, policy_rule, source_proto, process_name,
                 upstream_resolver_ms, trace_id, credential_ref
               FROM dns_events
               ORDER BY id DESC
               LIMIT 200`),
        query(`SELECT event_id, timestamp, action, path, size, trace_id, credential_ref
               FROM fs_events
               ORDER BY id DESC
               LIMIT 200`),
        query(`SELECT event_id, timestamp, exec_id, command, exit_code, duration_ms,
                 stdout_bytes, stderr_bytes, source, process_name, pid, trace_id,
                 credential_ref
               FROM exec_events
               ORDER BY id DESC
               LIMIT 100`),
        query(`SELECT event_id, timestamp, pid, ppid, uid, exe, comm, argv, cwd,
                 exit_code, session_id, tty, audit_id, exec_event_id, parent_exe,
                 trace_id, credential_ref
               FROM audit_events
               ORDER BY id DESC
               LIMIT 100`),
        query(`SELECT event_id, timestamp, material_class, source, event_type,
                 algorithm, substitution_ref, outcome, provider, confidence,
                 trace_id, context_json
               FROM substitution_events
               ORDER BY id DESC
               LIMIT 100`),
        api.getVmSecurityLatest(vmId, 200),
        api.getVmSecurityStatus(vmId),
        api.getVmDetectionLatest(vmId, 200),
        api.getVmEnforcementLatest(vmId, 200),
      ]);
      modelStats = modelStatsRows;
      modelRows = modelEventRows;
      mcpRows = mcpEventRows;
      httpRows = httpEventRows;
      dnsRows = dnsEventRows;
      fileRows = fsEventRows;
      processRows = processEventRows;
      auditRows = auditEventRows;
      substitutionRows = substitutionEventRows;
      securityLatest = secLatest;
      securityStatus = secStatus;
      detectionLatest = detLatest;
      enforcementLatest = enfLatest;
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to load session stats';
    } finally {
      loading = false;
    }
  }

  onMount(async () => {
    getShikiHighlighter().then(h => { shiki = h; });
    await load();
  });

  const modelCalls = $derived(modelStats.reduce((sum, row) => sum + number(row.call_count), 0));
  const modelInput = $derived(modelStats.reduce((sum, row) => sum + number(row.input_tokens), 0));
  const modelOutput = $derived(modelStats.reduce((sum, row) => sum + number(row.output_tokens), 0));
  const modelCost = $derived(modelStats.reduce((sum, row) => sum + number(row.estimated_cost_usd), 0));

  const mcpAllowed = $derived(mcpRows.filter(row => text(row.decision) === 'allowed').length);
  const mcpBlocked = $derived(mcpRows.filter(row => text(row.decision) !== 'allowed').length);
  const httpAllowed = $derived(httpRows.filter(row => text(row.decision) === 'allowed').length);
  const httpDenied = $derived(httpRows.filter(row => text(row.decision) !== 'allowed').length);
  const dnsDenied = $derived(dnsRows.filter(row => text(row.decision) !== 'allowed').length);
  const fileCreated = $derived(fileRows.filter(row => ['create', 'created'].includes(text(row.action))).length);
  const fileModified = $derived(fileRows.filter(row => ['modify', 'modified', 'write', 'written'].includes(text(row.action))).length);
  const fileDeleted = $derived(fileRows.filter(row => ['delete', 'deleted'].includes(text(row.action))).length);
  const processFailures = $derived(processRows.filter(row => row.exit_code != null && number(row.exit_code) !== 0).length);
  const processUniqueBinaries = $derived(new Set(auditRows.map(row => text(row.exe)).filter(Boolean)).size);

  function auditCommand(row: Row): string {
    return text(row.argv) || text(row.comm) || text(row.exe) || '--';
  }

  function brokerVerb(row: Row): string {
    const outcome = text(row.outcome).toLowerCase();
    if (outcome === 'brokered' || outcome === 'captured' || outcome === 'injected') return outcome;
    return 'captured';
  }

  function securityActionSummary(rows: api.SecurityRuleActionCount[] | undefined): Row[] {
    const counts = new Map<api.SecurityRuleAction, number>(SECURITY_ACTIONS.map(action => [action, 0]));
    for (const row of rows ?? []) {
      if (counts.has(row.rule_action)) counts.set(row.rule_action, number(row.count));
    }
    return SECURITY_ACTIONS.map(action => ({ rule_action: action, count: counts.get(action) ?? 0 }));
  }

  function securityDetectionSummary(rows: api.SecurityRuleStatsByRule[] | undefined): Row[] {
    const counts = new Map<api.RuntimeSecurityRuleDetectionLevel, number>(SECURITY_DETECTION_LEVELS.map(level => [level, 0]));
    for (const row of rows ?? []) {
      counts.set(row.detection_level, (counts.get(row.detection_level) ?? 0) + number(row.count));
    }
    return SECURITY_DETECTION_LEVELS.map(level => ({ detection_level: level, count: counts.get(level) ?? 0 }));
  }

  const brokerCapturedCount = $derived(substitutionRows.length);
  const brokerBrokeredCount = $derived(substitutionRows.filter(row => brokerVerb(row) === 'brokered').length);
  const brokerInjectedCount = $derived(substitutionRows.filter(row => brokerVerb(row) === 'injected').length);
  const detections = $derived(securityLatest.filter(row => row.detection_level !== 'none').length);
  const securityActionRows = $derived(securityActionSummary(securityStatus?.by_action));
  const securityDetectionRows = $derived(securityDetectionSummary(securityStatus?.by_rule));

  const navItems: { id: StatsTab; label: string; icon: any }[] = [
    { id: 'model', label: 'Model', icon: Brain },
    { id: 'mcp', label: 'MCP', icon: Wrench },
    { id: 'http', label: 'HTTP', icon: Globe },
    { id: 'dns', label: 'DNS', icon: DotsThreeCircle },
    { id: 'files', label: 'Files', icon: FileText },
    { id: 'process', label: 'Process', icon: Terminal },
    { id: 'credentials', label: 'Credentials', icon: Fingerprint },
    { id: 'security', label: 'Security', icon: ShieldCheck },
  ];
</script>

<div class="flex h-full">
  <aside class="w-56 shrink-0 border-e border-line-2 bg-background overflow-y-auto py-4">
    <div class="px-5 mb-4 flex items-center justify-between gap-x-2">
      <h1 class="text-xl font-bold text-foreground">Stats</h1>
      <button
        type="button"
        class="size-8 inline-flex items-center justify-center rounded-lg text-muted-foreground-1 hover:text-foreground hover:bg-muted-hover"
        onclick={inspectTab}
        title="Inspect session database"
        aria-label="Inspect session database"
      >
        <Database size={17} />
      </button>
    </div>
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

  <main class="flex-1 overflow-y-auto">
    <div class="py-6 px-8">
      <div class="flex items-center justify-between gap-x-3 mb-6">
        <div>
          <h2 class="text-xl font-medium text-foreground capitalize">{activeTab}</h2>
          <p class="text-xs text-muted-foreground-1 mt-1">Session {vmId} database</p>
        </div>
        <button
          type="button"
          class="inline-flex items-center gap-x-2 px-3 py-1.5 text-sm rounded-lg bg-layer border border-line-2 text-foreground hover:bg-muted-hover disabled:opacity-50"
          onclick={load}
          disabled={loading}
        >
          Refresh
        </button>
      </div>

      {#if error}
        <div class="p-4 mb-4 rounded-lg border border-destructive/30 bg-destructive/10 text-sm text-destructive">{error}</div>
      {/if}

      {#if activeTab === 'model'}
        <div class="grid grid-cols-4 gap-3 mb-6">
          <MetricCard label="Calls" value={modelCalls.toLocaleString()} />
          <MetricCard label="Input Tokens" value={modelInput.toLocaleString()} />
          <MetricCard label="Output Tokens" value={modelOutput.toLocaleString()} />
          <MetricCard label="Est. Cost" value={`$${modelCost.toFixed(2)}`} />
        </div>
        <StatsTable columns={['Provider', 'Model', 'Calls', 'Input', 'Output', 'Cost']} rows={modelStats}>
          {#snippet children(row: any)}
            <td class="px-4 py-2 text-foreground">{row.provider}</td>
            <td class="px-4 py-2 font-mono text-xs text-muted-foreground-1">{row.model}</td>
            <td class="px-4 py-2 text-right text-foreground">{number(row.call_count).toLocaleString()}</td>
            <td class="px-4 py-2 text-right text-foreground">{number(row.input_tokens).toLocaleString()}</td>
            <td class="px-4 py-2 text-right text-foreground">{number(row.output_tokens).toLocaleString()}</td>
            <td class="px-4 py-2 text-right text-foreground">${number(row.estimated_cost_usd).toFixed(2)}</td>
          {/snippet}
        </StatsTable>
        <StatsEventList title="Recent Model Events" rows={modelRows} columns={['Time', 'Provider', 'Model', 'Tokens', 'Trace']} onrow={(row) => detail = { type: 'model', data: row }}>
          {#snippet children(row: any)}
            <td class="px-4 py-2 text-muted-foreground">{formatTime(row.timestamp)}</td>
            <td class="px-4 py-2 text-foreground">{row.provider}</td>
            <td class="px-4 py-2 font-mono text-xs text-muted-foreground-1">{row.model ?? '--'}</td>
            <td class="px-4 py-2 text-right text-foreground">{number(row.input_tokens) + number(row.output_tokens)}</td>
            <td class="px-4 py-2 font-mono text-xs text-muted-foreground-1">{row.trace_id ?? '--'}</td>
          {/snippet}
        </StatsEventList>

      {:else if activeTab === 'mcp'}
        <div class="grid grid-cols-4 gap-3 mb-6">
          <MetricCard label="MCP Events" value={mcpRows.length.toLocaleString()} />
          <MetricCard label="Allowed" value={mcpAllowed.toLocaleString()} tone="primary" />
          <MetricCard label="Blocked/Error" value={mcpBlocked.toLocaleString()} tone="danger" />
          <MetricCard label="Credential Refs" value={mcpRows.filter(row => row.credential_ref).length.toLocaleString()} />
        </div>
        <StatsEventList title="MCP Events" rows={mcpRows} columns={['Time', 'Server', 'Method', 'Tool', 'Decision']} onrow={(row) => detail = { type: 'mcp', data: row }}>
          {#snippet children(row: any)}
            <td class="px-4 py-2 text-muted-foreground">{formatTime(row.timestamp)}</td>
            <td class="px-4 py-2 text-foreground">{row.server_name}</td>
            <td class="px-4 py-2 font-mono text-xs text-muted-foreground-1">{row.method}</td>
            <td class="px-4 py-2 font-mono text-xs text-foreground">{row.tool_name ?? '--'}</td>
            <td class="px-4 py-2"><StatsBadge value={text(row.decision)} kind="decision" /></td>
          {/snippet}
        </StatsEventList>

      {:else if activeTab === 'http'}
        <div class="grid grid-cols-4 gap-3 mb-6">
          <MetricCard label="HTTP Requests" value={httpRows.length.toLocaleString()} />
          <MetricCard label="Allowed" value={httpAllowed.toLocaleString()} tone="primary" />
          <MetricCard label="Denied/Error" value={httpDenied.toLocaleString()} tone="danger" />
          <MetricCard label="Bytes In" value={formatBytes(httpRows.reduce((sum, row) => sum + number(row.bytes_received), 0))} />
        </div>
        <StatsEventList title="HTTP Events" rows={httpRows} columns={['Time', 'Method', 'Host', 'Status', 'Decision']} onrow={(row) => detail = { type: 'http', data: row }}>
          {#snippet children(row: any)}
            <td class="px-4 py-2 text-muted-foreground">{formatTime(row.timestamp)}</td>
            <td class="px-4 py-2 font-mono text-xs font-semibold text-foreground">{row.method ?? 'CONNECT'}</td>
            <td class="px-4 py-2 font-mono text-xs text-muted-foreground-1 max-w-72 truncate">{row.domain}{row.path ?? ''}</td>
            <td class="px-4 py-2 text-center text-foreground">{row.status_code ?? '--'}</td>
            <td class="px-4 py-2"><StatsBadge value={text(row.decision)} kind="decision" /></td>
          {/snippet}
        </StatsEventList>

      {:else if activeTab === 'dns'}
        <div class="grid grid-cols-4 gap-3 mb-6">
          <MetricCard label="DNS Queries" value={dnsRows.length.toLocaleString()} />
          <MetricCard label="Denied/Error" value={dnsDenied.toLocaleString()} tone="danger" />
          <MetricCard label="Redirected" value={dnsRows.filter(row => text(row.decision) === 'redirected').length.toLocaleString()} />
          <MetricCard label="Avg Upstream" value={`${Math.round(dnsRows.reduce((sum, row) => sum + number(row.upstream_resolver_ms), 0) / Math.max(1, dnsRows.length))}ms`} />
        </div>
        <StatsEventList title="DNS Events" rows={dnsRows} columns={['Time', 'Name', 'Type', 'Rcode', 'Decision']} onrow={(row) => detail = { type: 'dns', data: row }}>
          {#snippet children(row: any)}
            <td class="px-4 py-2 text-muted-foreground">{formatTime(row.timestamp)}</td>
            <td class="px-4 py-2 font-mono text-xs text-foreground">{row.qname}</td>
            <td class="px-4 py-2 text-muted-foreground-1">{row.qtype}</td>
            <td class="px-4 py-2 text-muted-foreground-1">{row.rcode}</td>
            <td class="px-4 py-2"><StatsBadge value={text(row.decision)} kind="decision" /></td>
          {/snippet}
        </StatsEventList>

      {:else if activeTab === 'files'}
        <div class="grid grid-cols-4 gap-3 mb-6">
          <MetricCard label="File Events" value={fileRows.length.toLocaleString()} />
          <MetricCard label="Created" value={fileCreated.toLocaleString()} />
          <MetricCard label="Modified" value={fileModified.toLocaleString()} />
          <MetricCard label="Deleted" value={fileDeleted.toLocaleString()} tone="danger" />
        </div>
        <StatsEventList title="File Events" rows={fileRows} columns={['Time', 'Action', 'Path', 'Size', 'Trace']} onrow={(row) => detail = { type: 'file', data: row }}>
          {#snippet children(row: any)}
            <td class="px-4 py-2 text-muted-foreground">{formatTime(row.timestamp)}</td>
            <td class="px-4 py-2"><StatsBadge value={text(row.action)} /></td>
            <td class="px-4 py-2 font-mono text-xs text-foreground">{row.path}</td>
            <td class="px-4 py-2 text-right text-muted-foreground">{row.size != null ? formatBytes(number(row.size)) : '--'}</td>
            <td class="px-4 py-2 font-mono text-xs text-muted-foreground-1">{row.trace_id ?? '--'}</td>
          {/snippet}
        </StatsEventList>

      {:else if activeTab === 'process'}
        <div class="grid grid-cols-4 gap-3 mb-6">
          <MetricCard label="Exec Events" value={processRows.length.toLocaleString()} />
          <MetricCard label="Failures" value={processFailures.toLocaleString()} tone="danger" />
          <MetricCard label="Observed Processes" value={auditRows.length.toLocaleString()} />
          <MetricCard label="Unique Binaries" value={processUniqueBinaries.toLocaleString()} />
        </div>
        <StatsEventList title="Process Exec Events" rows={processRows} columns={['Time', 'Source', 'Command', 'Exit', 'Duration']} onrow={(row) => detail = { type: 'process', data: row }}>
          {#snippet children(row: any)}
            <td class="px-4 py-2 text-muted-foreground">{formatTime(row.timestamp)}</td>
            <td class="px-4 py-2 text-muted-foreground-1">{row.source}</td>
            <td class="px-4 py-2 font-mono text-xs text-foreground max-w-xl truncate">{row.command}</td>
            <td class="px-4 py-2 text-center text-foreground">{row.exit_code ?? '--'}</td>
            <td class="px-4 py-2 text-right text-muted-foreground">{row.duration_ms != null ? formatDuration(number(row.duration_ms)) : '--'}</td>
          {/snippet}
        </StatsEventList>
        <StatsEventList title="Observed Processes" rows={auditRows} columns={['Observed', 'Executable', 'Command', 'PID', 'Parent']} onrow={(row) => detail = { type: 'observed process', data: row }}>
          {#snippet children(row: any)}
            <td class="px-4 py-2 text-muted-foreground">{formatTime(row.timestamp)}</td>
            <td class="px-4 py-2 font-mono text-xs text-foreground max-w-xl truncate">{row.exe}</td>
            <td class="px-4 py-2 font-mono text-xs text-muted-foreground-1 max-w-xl truncate">{auditCommand(row)}</td>
            <td class="px-4 py-2 text-muted-foreground-1">{row.pid}</td>
            <td class="px-4 py-2 font-mono text-xs text-muted-foreground-1">{row.parent_exe ?? '--'}</td>
          {/snippet}
        </StatsEventList>
      {:else if activeTab === 'credentials'}
        <div class="grid grid-cols-4 gap-3 mb-6">
          <MetricCard label="Broker Events" value={substitutionRows.length.toLocaleString()} />
          <MetricCard label="Captured" value={brokerCapturedCount.toLocaleString()} />
          <MetricCard label="Brokered" value={brokerBrokeredCount.toLocaleString()} />
          <MetricCard label="Injected" value={brokerInjectedCount.toLocaleString()} />
        </div>
        <StatsEventList title="Credential Broker Events" rows={substitutionRows} columns={['Time', 'Verb', 'Source', 'Provider', 'Origin']} onrow={(row) => detail = { type: 'credential broker event', data: row }}>
          {#snippet children(row: any)}
            <td class="px-4 py-2 text-muted-foreground">{formatTime(row.timestamp)}</td>
            <td class="px-4 py-2"><StatsBadge value={brokerVerb(row)} /></td>
            <td class="px-4 py-2 text-muted-foreground-1">{row.source}</td>
            <td class="px-4 py-2 text-foreground">{row.provider ?? '--'}</td>
            <td class="px-4 py-2 font-mono text-xs text-muted-foreground-1">{row.event_type ?? '--'}</td>
          {/snippet}
        </StatsEventList>

      {:else if activeTab === 'security'}
        <div class="grid grid-cols-4 gap-3 mb-6">
          <MetricCard label="Rule Matches" value={(securityStatus?.total ?? securityLatest.length).toLocaleString()} />
          <MetricCard label="Detection Matches" value={detections.toLocaleString()} />
          <MetricCard label="Latest Detections" value={detectionLatest.length.toLocaleString()} />
          <MetricCard label="Latest Enforcement" value={enforcementLatest.length.toLocaleString()} />
        </div>
        {#if securityStatus}
          <div class="grid grid-cols-3 gap-4 mb-6">
            <StatsMiniGroup title="By Action" rows={securityActionRows} nameKey="rule_action" />
            <StatsMiniGroup title="By Detection Level" rows={securityDetectionRows} nameKey="detection_level" />
            <StatsMiniGroup title="By Event Type" rows={securityStatus.by_event_type} nameKey="event_type" />
          </div>
        {/if}
        <StatsEventList title="Security Ledger" rows={securityLatest} columns={['Time', 'Event', 'Rule', 'Action', 'Level']} onrow={(row) => detail = { type: 'security', data: row as any }}>
          {#snippet children(row: any)}
            <td class="px-4 py-2 text-muted-foreground">{formatTime(eventTimeMs(row.timestamp_unix_ms))}</td>
            <td class="px-4 py-2 font-mono text-xs text-foreground">{row.event_type}</td>
            <td class="px-4 py-2 font-mono text-xs text-muted-foreground-1">{row.rule_id}</td>
            <td class="px-4 py-2"><StatsBadge value={row.rule_action} /></td>
            <td class="px-4 py-2"><StatsBadge value={row.detection_level} kind="detection" /></td>
          {/snippet}
        </StatsEventList>
        <div class="grid grid-cols-2 gap-4">
          <StatsEventList title="Detection Latest" rows={detectionLatest} columns={['Time', 'Rule', 'Level']} onrow={(row) => detail = { type: 'detection', data: row as any }}>
            {#snippet children(row: any)}
              <td class="px-4 py-2 text-muted-foreground">{formatTime(eventTimeMs(row.timestamp_unix_ms))}</td>
              <td class="px-4 py-2 font-mono text-xs text-foreground">{row.rule_id}</td>
              <td class="px-4 py-2"><StatsBadge value={row.detection_level} kind="detection" /></td>
            {/snippet}
          </StatsEventList>
          <StatsEventList title="Enforcement Latest" rows={enforcementLatest} columns={['Time', 'Rule', 'Action']} onrow={(row) => detail = { type: 'enforcement', data: row as any }}>
            {#snippet children(row: any)}
              <td class="px-4 py-2 text-muted-foreground">{formatTime(eventTimeMs(row.timestamp_unix_ms))}</td>
              <td class="px-4 py-2 font-mono text-xs text-foreground">{row.rule_id}</td>
              <td class="px-4 py-2"><StatsBadge value={row.rule_action} /></td>
            {/snippet}
          </StatsEventList>
        </div>

      {/if}
    </div>
  </main>

  {#if detail}
    <div class="w-[460px] shrink-0 border-s border-line-2 flex flex-col overflow-hidden bg-background">
      <div class="flex items-center gap-2 px-3 py-2 border-b border-line-2 bg-surface">
        <span class="text-xs font-semibold flex-1 truncate capitalize text-foreground">{detail.type}</span>
        <button class="p-1 rounded hover:bg-muted-hover text-muted-foreground-1 hover:text-foreground" onclick={() => detail = null} aria-label="Close detail panel">
          <svg class="size-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
        </button>
      </div>
      <div class="flex-1 overflow-auto p-3 text-xs space-y-3">
        <div class="space-y-1">
          {#each visibleDetailEntries(detail.data) as [key, value]}
            <div class="grid grid-cols-[130px_1fr] gap-x-2">
              <span class="text-muted-foreground">{key}</span>
              <span class="font-mono text-foreground break-all">{formatDetailValue(value)}</span>
            </div>
          {/each}
        </div>
        {#each detailPayloadSections(detail.data) as section (section.key)}
          <div>
            <div class="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider mb-1">{section.label}</div>
            <div class="detail-shiki rounded overflow-auto max-h-80 bg-background-1">{@html formatAndHighlight(section.value, section.lang)}</div>
          </div>
        {/each}
        {#if detail.type === 'security' || detail.type === 'detection' || detail.type === 'enforcement'}
          <div>
            <div class="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider mb-1">Rule Snapshot</div>
            <div class="detail-shiki rounded overflow-auto max-h-64 bg-background-1">{@html formatAndHighlight(detail.data.rule_json, 'json')}</div>
          </div>
          <div>
            <div class="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider mb-1">Matched Event</div>
            <div class="detail-shiki rounded overflow-auto max-h-80 bg-background-1">{@html formatAndHighlight(detail.data.event_json, 'json')}</div>
          </div>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .detail-shiki :global(pre) {
    margin: 0;
    padding: 0.75rem;
    background: transparent !important;
    font-size: 11px;
    line-height: 1.5;
  }
</style>
