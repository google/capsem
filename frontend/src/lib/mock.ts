// Mock data for browser-only dev mode (no Tauri backend).
// Active when window.__TAURI_INTERNALS__ is absent.
//
// All DB-backed mock data comes from data/fixtures/test.db via sql.js.
// Static data (settings, VM state, session history) stays inline since
// those tables don't exist in the session DB.
import initSqlJs, { type Database } from 'sql.js';
import type {
  FileEvent,
  GlobalStats,
  McpCall,
  McpToolSummary,
  ModelCallResponse,
  NetEvent,
  ProviderSummary,
  QueryResult,
  ResolvedSetting,
  SessionInfo,
  SessionRecord,
  ToolSummary,
  TraceDetail,
  TraceSummary,
  VmStateResponse,
  GuestConfigResponse,
  NetworkPolicyResponse,
} from './types';

export const isMock = typeof window !== 'undefined' && !(window as any).__TAURI_INTERNALS__;

// ---------------------------------------------------------------------------
// sql.js singleton -- lazy-loaded on first DB query
// ---------------------------------------------------------------------------

let dbPromise: Promise<Database> | null = null;

function getDb(): Promise<Database> {
  if (!dbPromise) {
    dbPromise = (async () => {
      const SQL = await initSqlJs({
        locateFile: (file: string) => `/${file}`,
      });
      const resp = await fetch('/fixtures/test.db');
      const buf = await resp.arrayBuffer();
      return new SQL.Database(new Uint8Array(buf));
    })();
  }
  return dbPromise;
}

/** Run a SQL query against the fixture DB and return columnar JSON. */
async function queryFixture(sql: string): Promise<QueryResult> {
  const db = await getDb();
  const stmt = db.prepare(sql);
  const columns: string[] = stmt.getColumnNames();
  const rows: unknown[][] = [];
  while (stmt.step()) {
    rows.push(stmt.get());
  }
  stmt.free();
  return { columns, rows };
}

// ---------------------------------------------------------------------------
// Helpers to reshape columnar QueryResult into typed arrays
// ---------------------------------------------------------------------------

function rowsToObjects<T>(qr: QueryResult): T[] {
  return qr.rows.map((row) => {
    const obj: Record<string, unknown> = {};
    for (let i = 0; i < qr.columns.length; i++) {
      obj[qr.columns[i]] = row[i];
    }
    return obj as T;
  });
}

// ---------------------------------------------------------------------------
// DB-backed mock functions
// ---------------------------------------------------------------------------

async function mockNetEvents(_limit?: number, search?: string): Promise<NetEvent[]> {
  let sql = `SELECT CAST(strftime('%s', timestamp) AS INTEGER) AS timestamp,
                    domain, port, decision, process_name, pid,
                    method, path, query, status_code,
                    bytes_sent, bytes_received, duration_ms, matched_rule,
                    request_headers, response_headers,
                    request_body_preview, response_body_preview, conn_type
             FROM net_events`;
  if (search) {
    const q = search.replace(/'/g, "''");
    sql += ` WHERE domain LIKE '%${q}%'
                OR path LIKE '%${q}%'
                OR method LIKE '%${q}%'
                OR matched_rule LIKE '%${q}%'`;
  }
  sql += ` ORDER BY id DESC LIMIT ${_limit ?? 200}`;
  const qr = await queryFixture(sql);
  return rowsToObjects<NetEvent>(qr);
}

async function mockModelCalls(_limit?: number, _search?: string): Promise<ModelCallResponse[]> {
  let sql = `SELECT id, CAST(strftime('%s', timestamp) AS INTEGER) AS timestamp, provider, model, process_name, pid,
                    method, path, stream, system_prompt_preview,
                    messages_count, tools_count, request_bytes,
                    request_body_preview, message_id, status_code,
                    text_content, thinking_content, stop_reason,
                    input_tokens, output_tokens, usage_details, duration_ms,
                    response_bytes, estimated_cost_usd, trace_id
             FROM model_calls`;
  if (_search) {
    const q = _search.replace(/'/g, "''");
    sql += ` WHERE provider LIKE '%${q}%' OR model LIKE '%${q}%' OR stop_reason LIKE '%${q}%'`;
  }
  sql += ` ORDER BY id DESC LIMIT ${_limit ?? 50}`;
  const qr = await queryFixture(sql);

  // Load tool_calls and tool_responses for each model call
  const db = await getDb();
  return qr.rows.map((row) => {
    const obj: Record<string, unknown> = {};
    for (let i = 0; i < qr.columns.length; i++) {
      obj[qr.columns[i]] = row[i];
    }
    obj['stream'] = obj['stream'] !== 0;
    // Parse usage_details JSON string into object
    const udRaw = obj['usage_details'];
    obj['usage_details'] = (typeof udRaw === 'string' && udRaw) ? JSON.parse(udRaw) : {};
    const mcId = obj['id'] as number;

    // tool_calls
    const tcStmt = db.prepare('SELECT call_index, call_id, tool_name, arguments FROM tool_calls WHERE model_call_id = ? ORDER BY call_index');
    tcStmt.bind([mcId]);
    const toolCalls: unknown[] = [];
    while (tcStmt.step()) {
      const r = tcStmt.getAsObject();
      toolCalls.push(r);
    }
    tcStmt.free();
    obj['tool_calls'] = toolCalls;

    // tool_responses
    const trStmt = db.prepare('SELECT call_id, content_preview, is_error FROM tool_responses WHERE model_call_id = ?');
    trStmt.bind([mcId]);
    const toolResponses: unknown[] = [];
    while (trStmt.step()) {
      const r = trStmt.getAsObject();
      toolResponses.push({ ...r, is_error: r['is_error'] !== 0 });
    }
    trStmt.free();
    obj['tool_responses'] = toolResponses;

    return obj as unknown as ModelCallResponse;
  });
}

async function mockTraces(_limit?: number): Promise<TraceSummary[]> {
  const qr = await queryFixture(
    `SELECT trace_id,
            CAST(strftime('%s', MIN(timestamp)) AS INTEGER) as started_at,
            CAST(strftime('%s', MAX(timestamp)) AS INTEGER) as ended_at,
            (SELECT provider FROM model_calls m2 WHERE m2.trace_id = model_calls.trace_id ORDER BY m2.id ASC LIMIT 1) as provider,
            (SELECT model FROM model_calls m3 WHERE m3.trace_id = model_calls.trace_id ORDER BY m3.id ASC LIMIT 1) as model,
            COUNT(*) as call_count,
            COALESCE(SUM(COALESCE(input_tokens, 0)), 0) as total_input_tokens,
            COALESCE(SUM(COALESCE(output_tokens, 0)), 0) as total_output_tokens,
            COALESCE(SUM(duration_ms), 0) as total_duration_ms,
            COALESCE(SUM(estimated_cost_usd), 0.0) as total_estimated_cost_usd,
            (SELECT json_group_object(je.key, je.total) FROM (
                SELECT je.key, SUM(je.value) as total
                FROM model_calls mc6, json_each(mc6.usage_details) je
                WHERE mc6.trace_id = model_calls.trace_id AND mc6.usage_details IS NOT NULL
                GROUP BY je.key
            ) je) as total_usage_details,
            (SELECT COUNT(*) FROM tool_calls tc JOIN model_calls mc ON tc.model_call_id = mc.id WHERE mc.trace_id = model_calls.trace_id) as total_tool_calls,
            (SELECT stop_reason FROM model_calls m4 WHERE m4.trace_id = model_calls.trace_id ORDER BY m4.id DESC LIMIT 1) as stop_reason,
            (SELECT system_prompt_preview FROM model_calls m5 WHERE m5.trace_id = model_calls.trace_id ORDER BY m5.id ASC LIMIT 1) as system_prompt_preview
     FROM model_calls WHERE trace_id IS NOT NULL
     GROUP BY trace_id ORDER BY MAX(id) DESC LIMIT ${_limit ?? 50}`
  );
  const traces = rowsToObjects<any>(qr);

  for (const trace of traces) {
    if (typeof trace.total_usage_details === 'string') {
      try {
        trace.total_usage_details = JSON.parse(trace.total_usage_details);
      } catch {
        trace.total_usage_details = {};
      }
    } else {
      trace.total_usage_details = {};
    }
  }

  return traces as TraceSummary[];
}

async function mockTraceDetail(traceId: string): Promise<TraceDetail> {
  const calls = await mockModelCalls(100);
  const filtered = calls
    .filter((c) => c.trace_id === traceId)
    .sort((a, b) => a.id - b.id);
  return { trace_id: traceId, calls: filtered };
}

async function mockSessionInfo(): Promise<SessionInfo> {
  const db = await getDb();
  const netRow = db.exec(`SELECT COUNT(*),
      COALESCE(SUM(CASE WHEN decision = 'allowed' THEN 1 ELSE 0 END), 0),
      COALESCE(SUM(CASE WHEN decision = 'denied' THEN 1 ELSE 0 END), 0),
      COALESCE(SUM(CASE WHEN decision = 'error' THEN 1 ELSE 0 END), 0),
      COALESCE(SUM(bytes_sent), 0),
      COALESCE(SUM(bytes_received), 0)
    FROM net_events`)[0].values[0];
  const modelRow = db.exec(`SELECT COUNT(*),
      COALESCE(SUM(COALESCE(input_tokens, 0)), 0),
      COALESCE(SUM(COALESCE(output_tokens, 0)), 0),
      COALESCE(SUM(estimated_cost_usd), 0.0)
    FROM model_calls`)[0].values[0];
  const toolCount = db.exec('SELECT COUNT(*) FROM tool_calls')[0].values[0][0] as number;
  return {
    session_id: '20260225-143052-a7f3',
    mode: 'gui',
    uptime_ms: 45000,
    scratch_disk_size_gb: 8,
    ram_bytes: 512 * 1024 * 1024,
    total_requests: netRow[0] as number,
    allowed_requests: netRow[1] as number,
    denied_requests: netRow[2] as number,
    error_requests: netRow[3] as number,
    bytes_sent: netRow[4] as number,
    bytes_received: netRow[5] as number,
    model_call_count: modelRow[0] as number,
    total_input_tokens: modelRow[1] as number,
    total_output_tokens: modelRow[2] as number,
    total_usage_details: {},
    total_tool_calls: toolCount,
    total_estimated_cost_usd: modelRow[3] as number,
  };
}

// ---------------------------------------------------------------------------
// File events mock data (DB-backed with inline fallback)
// ---------------------------------------------------------------------------

const MOCK_FILE_EVENTS: FileEvent[] = [
  { timestamp: 1740502200, action: 'created', path: 'src/main.rs', size: 1240 },
  { timestamp: 1740502205, action: 'created', path: 'src/lib.rs', size: 580 },
  { timestamp: 1740502210, action: 'modified', path: 'Cargo.toml', size: 420 },
  { timestamp: 1740502220, action: 'created', path: 'src/config.rs', size: 2100 },
  { timestamp: 1740502225, action: 'modified', path: 'src/main.rs', size: 1580 },
  { timestamp: 1740502230, action: 'created', path: 'tests/integration.rs', size: 890 },
  { timestamp: 1740502240, action: 'deleted', path: 'src/old_module.rs', size: null },
  { timestamp: 1740502245, action: 'modified', path: 'src/config.rs', size: 2340 },
  { timestamp: 1740502250, action: 'created', path: 'README.md', size: 3200 },
  { timestamp: 1740502260, action: 'modified', path: 'src/lib.rs', size: 920 },
  { timestamp: 1740502270, action: 'created', path: 'src/utils.rs', size: 450 },
  { timestamp: 1740502280, action: 'deleted', path: 'src/temp.rs', size: null },
  { timestamp: 1740502290, action: 'modified', path: 'README.md', size: 3450 },
];

async function mockFileEvents(_limit?: number, search?: string): Promise<FileEvent[]> {
  // Try DB first
  try {
    const db = await getDb();
    const check = db.exec("SELECT name FROM sqlite_master WHERE type='table' AND name='fs_events'");
    if (check.length > 0) {
      let sql = `SELECT CAST(strftime('%s', timestamp) AS INTEGER) AS timestamp,
                        action, path, size
                 FROM fs_events`;
      if (search) {
        const q = search.replace(/'/g, "''");
        sql += ` WHERE path LIKE '%${q}%'`;
      }
      sql += ` ORDER BY id DESC LIMIT ${_limit ?? 200}`;
      const qr = await queryFixture(sql);
      if (qr.rows.length > 0) return rowsToObjects<FileEvent>(qr);
    }
  } catch { /* fallback to inline */ }

  let events = [...MOCK_FILE_EVENTS];
  if (search) {
    const q = search.toLowerCase();
    events = events.filter(e => e.path.toLowerCase().includes(q));
  }
  return events.slice(0, _limit ?? 200);
}

// ---------------------------------------------------------------------------
// MCP mock data
// ---------------------------------------------------------------------------

const MOCK_MCP_CALLS: McpCall[] = [
  {
    timestamp: 1740502200, server_name: 'github', method: 'tools/list',
    tool_name: null, request_id: '1', request_preview: null,
    response_preview: '{"tools":[{"name":"github__search_repos",...}]}',
    decision: 'allowed', duration_ms: 42, error_message: null, process_name: 'claude',
  },
  {
    timestamp: 1740502210, server_name: 'github', method: 'tools/call',
    tool_name: 'github__search_repos', request_id: '2',
    request_preview: '{"name":"github__search_repos","arguments":{"query":"capsem"}}',
    response_preview: '{"content":[{"type":"text","text":"Found 3 repos"}]}',
    decision: 'allowed', duration_ms: 350, error_message: null, process_name: 'claude',
  },
  {
    timestamp: 1740502230, server_name: 'github', method: 'tools/call',
    tool_name: 'github__create_issue', request_id: '3',
    request_preview: '{"name":"github__create_issue","arguments":{"title":"Bug fix"}}',
    response_preview: null,
    decision: 'warned', duration_ms: 180, error_message: null, process_name: 'claude',
  },
  {
    timestamp: 1740502250, server_name: 'filesystem', method: 'tools/call',
    tool_name: 'filesystem__read_file', request_id: '4',
    request_preview: '{"name":"filesystem__read_file","arguments":{"path":"/etc/passwd"}}',
    response_preview: null,
    decision: 'denied', duration_ms: 1, error_message: 'tool blocked by policy: filesystem__read_file',
    process_name: 'gemini',
  },
  {
    timestamp: 1740502260, server_name: 'filesystem', method: 'tools/call',
    tool_name: 'filesystem__write_file', request_id: '5',
    request_preview: '{"name":"filesystem__write_file","arguments":{"path":"/tmp/out.txt"}}',
    response_preview: '{"content":[{"type":"text","text":"Written 42 bytes"}]}',
    decision: 'allowed', duration_ms: 12, error_message: null, process_name: 'gemini',
  },
  {
    timestamp: 1740502280, server_name: 'slack', method: 'tools/call',
    tool_name: 'slack__send_message', request_id: '6',
    request_preview: '{"name":"slack__send_message","arguments":{"channel":"general"}}',
    response_preview: null,
    decision: 'error', duration_ms: 5020, error_message: 'MCP server not running: slack',
    process_name: 'claude',
  },
];

async function mockMcpCalls(_limit?: number, _search?: string): Promise<McpCall[]> {
  let calls = [...MOCK_MCP_CALLS];
  if (_search) {
    const q = _search.toLowerCase();
    calls = calls.filter(c =>
      c.server_name.toLowerCase().includes(q) ||
      (c.tool_name && c.tool_name.toLowerCase().includes(q)) ||
      c.method.toLowerCase().includes(q)
    );
  }
  return calls.slice(0, _limit ?? 50);
}

// ---------------------------------------------------------------------------
// Static mock data (not in session DB)
// ---------------------------------------------------------------------------

// Helper: creates a default mock setting with sensible defaults for empty fields.
function ms(overrides: Partial<ResolvedSetting> & { id: string; category: string; name: string; setting_type: ResolvedSetting['setting_type'] }): ResolvedSetting {
  return {
    description: '',
    default_value: overrides.setting_type === 'bool' ? false : overrides.setting_type === 'number' ? 0 : '',
    effective_value: overrides.setting_type === 'bool' ? false : overrides.setting_type === 'number' ? 0 : '',
    source: 'default',
    modified: null,
    corp_locked: false,
    enabled_by: null,
    enabled: true,
    metadata: { domains: [], choices: [], min: null, max: null, rules: {} },
    ...overrides,
  };
}

const MOCK_SETTINGS: ResolvedSetting[] = [
  // -- AI Providers --
  ms({
    id: 'ai.anthropic.allow', category: 'AI Providers', name: 'Allow Anthropic', setting_type: 'bool',
    description: 'Enable API access to Anthropic (api.anthropic.com).',
    default_value: false, effective_value: false,
  }),
  ms({
    id: 'ai.anthropic.api_key', category: 'AI Providers', name: 'Anthropic API Key', setting_type: 'apikey',
    description: 'API key for Anthropic. Injected as ANTHROPIC_API_KEY env var.',
    default_value: '', effective_value: '', enabled_by: 'ai.anthropic.allow', enabled: false,
  }),
  ms({
    id: 'ai.anthropic.domains', category: 'AI Providers', name: 'Anthropic Domains', setting_type: 'text',
    description: 'Comma-separated domain patterns.',
    default_value: '*.anthropic.com, *.claude.com', effective_value: '*.anthropic.com, *.claude.com',
    enabled_by: 'ai.anthropic.allow', enabled: false,
  }),
  ms({
    id: 'ai.anthropic.claude.settings_json', category: 'AI Providers', name: 'Claude Code settings.json', setting_type: 'file',
    description: 'Content for ~/.claude/settings.json.',
    default_value: '{"permissions":{"defaultMode":"bypassPermissions"},"env":{"CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC":"1"}}',
    effective_value: '{"permissions":{"defaultMode":"bypassPermissions"},"env":{"CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC":"1"}}',
    enabled_by: 'ai.anthropic.allow', enabled: false,
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, guest_path: '/root/.claude/settings.json' },
  }),
  ms({
    id: 'ai.anthropic.claude.state_json', category: 'AI Providers', name: 'Claude Code state (.claude.json)', setting_type: 'file',
    description: 'Content for ~/.claude.json. Skips onboarding.',
    default_value: '{"hasCompletedOnboarding":true,"hasTrustDialogAccepted":true}',
    effective_value: '{"hasCompletedOnboarding":true,"hasTrustDialogAccepted":true}',
    enabled_by: 'ai.anthropic.allow', enabled: false,
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, guest_path: '/root/.claude.json' },
  }),
  ms({
    id: 'ai.openai.allow', category: 'AI Providers', name: 'Allow OpenAI', setting_type: 'bool',
    description: 'Enable API access to OpenAI (api.openai.com).',
    default_value: false, effective_value: false,
  }),
  ms({
    id: 'ai.openai.api_key', category: 'AI Providers', name: 'OpenAI API Key', setting_type: 'apikey',
    description: 'API key for OpenAI. Injected as OPENAI_API_KEY env var.',
    default_value: '', effective_value: '', enabled_by: 'ai.openai.allow', enabled: false,
  }),
  ms({
    id: 'ai.openai.domains', category: 'AI Providers', name: 'OpenAI Domains', setting_type: 'text',
    description: 'Comma-separated domain patterns.',
    default_value: '*.openai.com', effective_value: '*.openai.com',
    enabled_by: 'ai.openai.allow', enabled: false,
  }),
  ms({
    id: 'ai.google.allow', category: 'AI Providers', name: 'Allow Google AI', setting_type: 'bool',
    description: 'Enable API access to Google AI (*.googleapis.com).',
    default_value: true, effective_value: true,
  }),
  ms({
    id: 'ai.google.api_key', category: 'AI Providers', name: 'Google AI API Key', setting_type: 'apikey',
    description: 'API key for Google AI. Injected as GEMINI_API_KEY env var.',
    default_value: '', effective_value: '', enabled_by: 'ai.google.allow',
  }),
  ms({
    id: 'ai.google.domains', category: 'AI Providers', name: 'Google AI Domains', setting_type: 'text',
    description: 'Comma-separated domain patterns.',
    default_value: '*.googleapis.com', effective_value: '*.googleapis.com',
    enabled_by: 'ai.google.allow',
  }),
  ms({
    id: 'ai.google.gemini.settings_json', category: 'AI Providers', name: 'Gemini settings.json', setting_type: 'file',
    description: 'Content for ~/.gemini/settings.json.',
    default_value: '{"approvalMode":"yolo","general":{"enableAutoUpdate":false}}',
    effective_value: '{"approvalMode":"yolo","general":{"enableAutoUpdate":false}}',
    enabled_by: 'ai.google.allow',
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, guest_path: '/root/.gemini/settings.json' },
  }),
  ms({
    id: 'ai.google.gemini.projects_json', category: 'AI Providers', name: 'Gemini projects.json', setting_type: 'file',
    description: 'Content for ~/.gemini/projects.json.',
    default_value: '{"projects":{"/root":"root"}}',
    effective_value: '{"projects":{"/root":"root"}}',
    enabled_by: 'ai.google.allow',
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, guest_path: '/root/.gemini/projects.json' },
  }),
  ms({
    id: 'ai.google.gemini.trusted_folders_json', category: 'AI Providers', name: 'Gemini trustedFolders.json', setting_type: 'file',
    description: 'Content for ~/.gemini/trustedFolders.json.',
    default_value: '{"/root":"TRUST_FOLDER"}',
    effective_value: '{"/root":"TRUST_FOLDER"}',
    enabled_by: 'ai.google.allow',
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, guest_path: '/root/.gemini/trustedFolders.json' },
  }),
  ms({
    id: 'ai.google.gemini.installation_id', category: 'AI Providers', name: 'Gemini installation_id', setting_type: 'text',
    description: 'Stable UUID avoids first-run prompts.',
    default_value: 'capsem-sandbox-00000000-0000-0000-0000-000000000000',
    effective_value: 'capsem-sandbox-00000000-0000-0000-0000-000000000000',
    enabled_by: 'ai.google.allow',
    metadata: { domains: [], choices: [], min: null, max: null, rules: {}, guest_path: '/root/.gemini/installation_id' },
  }),
  // -- Search --
  ms({
    id: 'search.google.allow', category: 'Search', name: 'Allow Google Search', setting_type: 'bool',
    description: 'Enable access to Google web search.',
    default_value: true, effective_value: true,
    metadata: { domains: ['www.google.com', 'google.com'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'search.perplexity.allow', category: 'Search', name: 'Allow Perplexity', setting_type: 'bool',
    description: 'Enable access to Perplexity AI search.',
    default_value: false, effective_value: false,
    metadata: { domains: ['perplexity.ai', '*.perplexity.ai'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'search.firecrawl.allow', category: 'Search', name: 'Allow Firecrawl', setting_type: 'bool',
    description: 'Enable access to Firecrawl web scraping API.',
    default_value: false, effective_value: false,
    metadata: { domains: ['firecrawl.dev', 'api.firecrawl.dev'], choices: [], min: null, max: null, rules: {} },
  }),
  // -- Package Registries --
  ms({
    id: 'registry.github.allow', category: 'Package Registries', name: 'Allow GitHub', setting_type: 'bool',
    description: 'Enable access to GitHub and GitHub-hosted content.',
    default_value: true, effective_value: true,
    metadata: { domains: ['github.com', '*.github.com', '*.githubusercontent.com'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'registry.npm.allow', category: 'Package Registries', name: 'Allow npm', setting_type: 'bool',
    description: 'Enable access to the npm package registry.',
    default_value: true, effective_value: true,
    metadata: { domains: ['registry.npmjs.org', '*.npmjs.org'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'registry.pypi.allow', category: 'Package Registries', name: 'Allow PyPI', setting_type: 'bool',
    description: 'Enable access to the Python Package Index.',
    default_value: true, effective_value: true,
    metadata: { domains: ['pypi.org', 'files.pythonhosted.org'], choices: [], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'registry.crates.allow', category: 'Package Registries', name: 'Allow crates.io', setting_type: 'bool',
    description: 'Enable access to the Rust crate registry.',
    default_value: true, effective_value: true,
    metadata: { domains: ['crates.io', 'static.crates.io'], choices: [], min: null, max: null, rules: {} },
  }),
  // -- Guest Environment --
  ms({
    id: 'guest.shell.term', category: 'Guest Environment', name: 'TERM', setting_type: 'text',
    description: 'Terminal type for the guest shell.',
    default_value: 'xterm-256color', effective_value: 'xterm-256color',
  }),
  ms({
    id: 'guest.shell.home', category: 'Guest Environment', name: 'HOME', setting_type: 'text',
    description: 'Home directory for the guest shell.',
    default_value: '/root', effective_value: '/root',
  }),
  ms({
    id: 'guest.shell.path', category: 'Guest Environment', name: 'PATH', setting_type: 'text',
    description: 'Executable search path for the guest shell.',
    default_value: '/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
    effective_value: '/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
  }),
  ms({
    id: 'guest.shell.lang', category: 'Guest Environment', name: 'LANG', setting_type: 'text',
    description: 'Locale for the guest shell.',
    default_value: 'C', effective_value: 'C',
  }),
  ms({
    id: 'guest.tls.ca_bundle', category: 'Guest Environment', name: 'CA bundle path', setting_type: 'text',
    description: 'Path to the CA certificate bundle in the guest.',
    default_value: '/etc/ssl/certs/ca-certificates.crt',
    effective_value: '/etc/ssl/certs/ca-certificates.crt',
  }),
  // -- Network --
  ms({
    id: 'network.default_action', category: 'Network', name: 'Default action', setting_type: 'text',
    description: 'Action for domains not in any allow/block list.',
    default_value: 'deny', effective_value: 'deny',
    metadata: { domains: [], choices: ['allow', 'deny'], min: null, max: null, rules: {} },
  }),
  ms({
    id: 'vm.log_bodies', category: 'VM', name: 'Log request bodies', setting_type: 'bool',
    description: 'Capture request/response bodies in telemetry.',
    default_value: false, effective_value: false,
  }),
  ms({
    id: 'vm.max_body_capture', category: 'VM', name: 'Max body capture', setting_type: 'number',
    description: 'Maximum bytes of body to capture in telemetry.',
    default_value: 4096, effective_value: 4096,
    metadata: { domains: [], choices: [], min: 0, max: 1048576, rules: {} },
  }),
  ms({
    id: 'network.custom_allow', category: 'Network', name: 'Custom allowed domains', setting_type: 'text',
    description: 'Comma-separated domain patterns to allow. Wildcards supported (*.example.com).',
    default_value: 'elie.net, *.elie.net', effective_value: 'elie.net, *.elie.net',
  }),
  ms({
    id: 'network.custom_block', category: 'Network', name: 'Custom blocked domains', setting_type: 'text',
    description: 'Comma-separated domain patterns to block. Takes priority over custom allow list.',
    default_value: '', effective_value: '',
  }),
  // -- Session (in VM category) --
  ms({
    id: 'vm.retention_days', category: 'VM', name: 'Session retention', setting_type: 'number',
    description: 'Number of days to retain session data.',
    default_value: 30, effective_value: 30,
    metadata: { domains: [], choices: [], min: 1, max: 365, rules: {} },
  }),
  ms({
    id: 'vm.max_sessions', category: 'VM', name: 'Maximum sessions', setting_type: 'number',
    description: 'Keep at most this many sessions (oldest culled first).',
    default_value: 100, effective_value: 100,
    metadata: { domains: [], choices: [], min: 1, max: 10000, rules: {} },
  }),
  ms({
    id: 'vm.max_disk_gb', category: 'VM', name: 'Maximum disk usage', setting_type: 'number',
    description: 'Maximum total disk usage for all sessions in GB.',
    default_value: 100, effective_value: 100,
    metadata: { domains: [], choices: [], min: 1, max: 1000, rules: {} },
  }),
  // -- Appearance --
  ms({
    id: 'appearance.dark_mode', category: 'Appearance', name: 'Dark mode', setting_type: 'bool',
    description: 'Use dark color scheme in the UI.',
    default_value: true, effective_value: true,
  }),
  ms({
    id: 'appearance.font_size', category: 'Appearance', name: 'Font size', setting_type: 'number',
    description: 'Terminal font size in pixels.',
    default_value: 14, effective_value: 14,
    metadata: { domains: [], choices: [], min: 8, max: 32, rules: {} },
  }),
  // -- VM --
  ms({
    id: 'vm.scratch_disk_size_gb', category: 'VM', name: 'Scratch disk size', setting_type: 'number',
    description: 'Size of the ephemeral scratch disk in GB.',
    default_value: 8, effective_value: 8,
    metadata: { domains: [], choices: [], min: 1, max: 128, rules: {} },
  }),
];

const MOCK_VM_STATE: VmStateResponse = {
  state: 'Running',
  elapsed_ms: 45000,
  history: [
    { from: 'Created', to: 'Booting', trigger: 'vm_started', duration_ms: 50 },
    { from: 'Booting', to: 'WaitingForAgent', trigger: 'kernel_boot', duration_ms: 1200 },
    { from: 'WaitingForAgent', to: 'Configuring', trigger: 'agent_connected', duration_ms: 800 },
    { from: 'Configuring', to: 'Running', trigger: 'boot_ready_received', duration_ms: 200 },
  ],
};

const MOCK_SESSION_HISTORY: SessionRecord[] = [
  {
    id: '20260225-143052-a7f3',
    mode: 'gui',
    command: null,
    status: 'running',
    created_at: '2026-02-25T14:30:52Z',
    stopped_at: null,
    scratch_disk_size_gb: 8,
    ram_bytes: 512 * 1024 * 1024,
    total_requests: 23,
    allowed_requests: 17,
    denied_requests: 6,
    total_input_tokens: 45200,
    total_output_tokens: 12800,
    total_estimated_cost: 0.42,
    total_tool_calls: 67,
    total_mcp_calls: 5,
    total_file_events: 23,
  },
  {
    id: '20260225-120000-b8e4',
    mode: 'cli',
    command: 'python3 train.py',
    status: 'stopped',
    created_at: '2026-02-25T12:00:00Z',
    stopped_at: '2026-02-25T13:45:20Z',
    scratch_disk_size_gb: 8,
    ram_bytes: 512 * 1024 * 1024,
    total_requests: 42,
    allowed_requests: 38,
    denied_requests: 4,
    total_input_tokens: 128000,
    total_output_tokens: 35000,
    total_estimated_cost: 1.85,
    total_tool_calls: 142,
    total_mcp_calls: 12,
    total_file_events: 89,
  },
  {
    id: '20260225-090000-c9d5',
    mode: 'gui',
    command: null,
    status: 'stopped',
    created_at: '2026-02-25T09:00:00Z',
    stopped_at: '2026-02-25T11:30:00Z',
    scratch_disk_size_gb: 8,
    ram_bytes: 512 * 1024 * 1024,
    total_requests: 105,
    allowed_requests: 92,
    denied_requests: 13,
    total_input_tokens: 256000,
    total_output_tokens: 78000,
    total_estimated_cost: 3.20,
    total_tool_calls: 310,
    total_mcp_calls: 28,
    total_file_events: 156,
  },
  {
    id: '20260224-160000-d0e6',
    mode: 'cli',
    command: 'npm test',
    status: 'crashed',
    created_at: '2026-02-24T16:00:00Z',
    stopped_at: null,
    scratch_disk_size_gb: 8,
    ram_bytes: 512 * 1024 * 1024,
    total_requests: 7,
    allowed_requests: 5,
    denied_requests: 2,
    total_input_tokens: 8500,
    total_output_tokens: 2200,
    total_estimated_cost: 0.08,
    total_tool_calls: 15,
    total_mcp_calls: 0,
    total_file_events: 4,
  },
];

// ---------------------------------------------------------------------------
// Cross-session dashboard stats (mock)
// ---------------------------------------------------------------------------

async function mockGlobalStats(): Promise<GlobalStats> {
  const sessions = MOCK_SESSION_HISTORY;
  return {
    total_sessions: sessions.length,
    total_input_tokens: sessions.reduce((s, r) => s + r.total_input_tokens, 0),
    total_output_tokens: sessions.reduce((s, r) => s + r.total_output_tokens, 0),
    total_estimated_cost: sessions.reduce((s, r) => s + r.total_estimated_cost, 0),
    total_tool_calls: sessions.reduce((s, r) => s + r.total_tool_calls, 0),
    total_mcp_calls: sessions.reduce((s, r) => s + r.total_mcp_calls, 0),
    total_file_events: sessions.reduce((s, r) => s + r.total_file_events, 0),
    total_requests: sessions.reduce((s, r) => s + r.total_requests, 0),
    total_allowed: sessions.reduce((s, r) => s + r.allowed_requests, 0),
    total_denied: sessions.reduce((s, r) => s + r.denied_requests, 0),
  };
}

async function mockTopProviders(_limit?: number): Promise<ProviderSummary[]> {
  return [
    { provider: 'anthropic', call_count: 85, input_tokens: 320000, output_tokens: 95000, estimated_cost: 3.80, total_duration_ms: 125000 },
    { provider: 'google', call_count: 42, input_tokens: 110000, output_tokens: 28000, estimated_cost: 1.65, total_duration_ms: 68000 },
    { provider: 'openai', call_count: 8, input_tokens: 7700, output_tokens: 5000, estimated_cost: 0.10, total_duration_ms: 12000 },
  ];
}

async function mockTopTools(_limit?: number): Promise<ToolSummary[]> {
  return [
    { tool_name: 'Read', call_count: 142, total_bytes: 850000, total_duration_ms: 28000 },
    { tool_name: 'Edit', call_count: 89, total_bytes: 420000, total_duration_ms: 35000 },
    { tool_name: 'Bash', call_count: 67, total_bytes: 1200000, total_duration_ms: 95000 },
    { tool_name: 'Write', call_count: 45, total_bytes: 380000, total_duration_ms: 18000 },
    { tool_name: 'Grep', call_count: 38, total_bytes: 120000, total_duration_ms: 8500 },
  ];
}

async function mockTopMcpTools(_limit?: number): Promise<McpToolSummary[]> {
  return [
    { tool_name: 'github__search_repos', server_name: 'github', call_count: 12, total_bytes: 45000, total_duration_ms: 4200 },
    { tool_name: 'filesystem__write_file', server_name: 'filesystem', call_count: 8, total_bytes: 12000, total_duration_ms: 960 },
    { tool_name: 'github__create_issue', server_name: 'github', call_count: 5, total_bytes: 8500, total_duration_ms: 2800 },
  ];
}

// ---------------------------------------------------------------------------
// Exported mock API
// ---------------------------------------------------------------------------

export const mockApi = {
  vmStatus: async () => 'running',
  serialInput: async (_input: string) => {},
  terminalResize: async (_cols: number, _rows: number) => {},
  netEvents: mockNetEvents,
  getGuestConfig: async (): Promise<GuestConfigResponse> => ({ env: { TERM: 'xterm-256color', HOME: '/root' } }),
  getNetworkPolicy: async (): Promise<NetworkPolicyResponse> => ({
    allow: [
      'github.com', '*.github.com', '*.githubusercontent.com',
      'registry.npmjs.org', '*.npmjs.org',
      'pypi.org', 'files.pythonhosted.org',
      'crates.io', 'static.crates.io',
      '*.googleapis.com',
      'www.google.com', 'google.com',
      'elie.net', '*.elie.net',
    ],
    block: [
      '*.anthropic.com', '*.claude.com',
      '*.openai.com',
      'perplexity.ai', '*.perplexity.ai',
      'firecrawl.dev', 'api.firecrawl.dev',
    ],
    default_action: 'deny',
    corp_managed: false,
    conflicts: [],
  }),
  setGuestEnv: async (_key: string, _value: string) => {},
  removeGuestEnv: async (_key: string) => {},
  getSettings: async () => MOCK_SETTINGS,
  updateSetting: async (_id: string, _value: any) => {},
  getVmState: async () => MOCK_VM_STATE,
  getSessionInfo: mockSessionInfo,
  getSessionHistory: async (_limit?: number) => MOCK_SESSION_HISTORY,
  getModelCalls: mockModelCalls,
  getTraces: mockTraces,
  getTraceDetail: mockTraceDetail,
  getFileEvents: mockFileEvents,
  getMcpCalls: mockMcpCalls,
  getGlobalStats: mockGlobalStats,
  getTopProviders: mockTopProviders,
  getTopTools: mockTopTools,
  getTopMcpTools: mockTopMcpTools,
  queryDb: queryFixture,

  // Event listeners return no-op unsubscribers in mock mode
  onSerialOutput: async (_cb: (data: number[]) => void) => () => {},
  onVmStateChanged: async (_cb: (state: string) => void) => () => {},
  onTerminalSourceChanged: async (_cb: (source: string) => void) => () => {},
};
