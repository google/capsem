// Mock data for browser-only dev mode (no Tauri backend).
// Active when window.__TAURI_INTERNALS__ is absent.
//
// All DB-backed mock data comes from data/fixtures/test.db via sql.js.
// Static data (settings, VM state, session history) stays inline since
// those tables don't exist in the session DB.
//
// SQL queries (session + main) go through queryFixture / queryFixtureMain,
// exported for use by db.ts.
import initSqlJs, { type Database } from 'sql.js';
import type {
  QueryResult,
  ResolvedSetting,
  SessionInfo,
  SessionRecord,
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
export async function queryFixture(sql: string, params?: unknown[]): Promise<QueryResult> {
  const db = await getDb();
  const stmt = db.prepare(sql);
  if (params && params.length > 0) {
    stmt.bind(params as any[]);
  }
  const columns: string[] = stmt.getColumnNames();
  const rows: unknown[][] = [];
  while (stmt.step()) {
    rows.push(stmt.get());
  }
  stmt.free();
  return { columns, rows };
}

/** Run a SQL query against main.db (mock: returns static data inline). */
export async function queryFixtureMain(sql: string, _params?: unknown[]): Promise<QueryResult> {
  // main.db tables (sessions, ai_usage, tool_usage, mcp_usage) don't exist in
  // the fixture DB. Return static mock data based on the query pattern.
  const upper = sql.trim().toUpperCase();

  if (upper.includes('FROM SESSIONS') && upper.includes('COUNT(*)')) {
    // GLOBAL_STATS_SQL
    const sessions = MOCK_SESSION_HISTORY;
    return {
      columns: ['total_sessions', 'total_input_tokens', 'total_output_tokens',
                 'total_estimated_cost', 'total_tool_calls', 'total_mcp_calls',
                 'total_file_events', 'total_requests', 'total_allowed', 'total_denied'],
      rows: [[
        sessions.length,
        sessions.reduce((s, r) => s + r.total_input_tokens, 0),
        sessions.reduce((s, r) => s + r.total_output_tokens, 0),
        sessions.reduce((s, r) => s + r.total_estimated_cost, 0),
        sessions.reduce((s, r) => s + r.total_tool_calls, 0),
        sessions.reduce((s, r) => s + r.total_mcp_calls, 0),
        sessions.reduce((s, r) => s + r.total_file_events, 0),
        sessions.reduce((s, r) => s + r.total_requests, 0),
        sessions.reduce((s, r) => s + r.allowed_requests, 0),
        sessions.reduce((s, r) => s + r.denied_requests, 0),
      ]],
    };
  }

  if (upper.includes('FROM SESSIONS')) {
    // SESSION_HISTORY_SQL
    return {
      columns: ['id', 'mode', 'command', 'status', 'created_at', 'stopped_at',
                 'scratch_disk_size_gb', 'ram_bytes',
                 'total_requests', 'allowed_requests', 'denied_requests',
                 'total_input_tokens', 'total_output_tokens', 'total_estimated_cost',
                 'total_tool_calls', 'total_mcp_calls', 'total_file_events',
                 'compressed_size_bytes', 'vacuumed_at'],
      rows: MOCK_SESSION_HISTORY.map(s => [
        s.id, s.mode, s.command, s.status, s.created_at, s.stopped_at,
        s.scratch_disk_size_gb, s.ram_bytes,
        s.total_requests, s.allowed_requests, s.denied_requests,
        s.total_input_tokens, s.total_output_tokens, s.total_estimated_cost,
        s.total_tool_calls, s.total_mcp_calls, s.total_file_events,
        s.compressed_size_bytes, s.vacuumed_at,
      ]),
    };
  }

  if (upper.includes('FROM AI_USAGE')) {
    // TOP_PROVIDERS_SQL
    const data = [
      ['anthropic', 85, 320000, 95000, 3.80, 125000],
      ['google', 42, 110000, 28000, 1.65, 68000],
      ['openai', 8, 7700, 5000, 0.10, 12000],
    ];
    return {
      columns: ['provider', 'call_count', 'input_tokens', 'output_tokens', 'estimated_cost', 'total_duration_ms'],
      rows: data,
    };
  }

  if (upper.includes('FROM TOOL_USAGE')) {
    // TOP_TOOLS_SQL
    const data = [
      ['Read', 142, 850000, 28000],
      ['Edit', 89, 420000, 35000],
      ['Bash', 67, 1200000, 95000],
      ['Write', 45, 380000, 18000],
      ['Grep', 38, 120000, 8500],
    ];
    return {
      columns: ['tool_name', 'call_count', 'total_bytes', 'total_duration_ms'],
      rows: data,
    };
  }

  if (upper.includes('FROM MCP_USAGE')) {
    // TOP_MCP_TOOLS_SQL
    const data = [
      ['github__search_repos', 'github', 12, 45000, 4200],
      ['filesystem__write_file', 'filesystem', 8, 12000, 960],
      ['github__create_issue', 'github', 5, 8500, 2800],
    ];
    return {
      columns: ['tool_name', 'server_name', 'call_count', 'total_bytes', 'total_duration_ms'],
      rows: data,
    };
  }

  // Fallback: empty result
  return { columns: [], rows: [] };
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
    compressed_size_bytes: null,
    vacuumed_at: null,
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
    compressed_size_bytes: null,
    vacuumed_at: null,
  },
  {
    id: '20260225-090000-c9d5',
    mode: 'gui',
    command: null,
    status: 'vacuumed',
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
    compressed_size_bytes: 245760,
    vacuumed_at: '2026-02-25T11:31:00Z',
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
    compressed_size_bytes: null,
    vacuumed_at: null,
  },
  {
    id: '20260223-100000-e1f7',
    mode: 'gui',
    command: null,
    status: 'terminated',
    created_at: '2026-02-23T10:00:00Z',
    stopped_at: '2026-02-23T12:00:00Z',
    scratch_disk_size_gb: 8,
    ram_bytes: 512 * 1024 * 1024,
    total_requests: 52,
    allowed_requests: 45,
    denied_requests: 7,
    total_input_tokens: 98000,
    total_output_tokens: 32000,
    total_estimated_cost: 1.15,
    total_tool_calls: 85,
    total_mcp_calls: 6,
    total_file_events: 42,
    compressed_size_bytes: 189000,
    vacuumed_at: '2026-02-23T12:01:00Z',
  },
];

// ---------------------------------------------------------------------------
// Exported mock API (non-SQL commands only; SQL goes through queryFixture)
// ---------------------------------------------------------------------------

export const mockApi = {
  vmStatus: async () => 'running',
  serialInput: async (_input: string) => {},
  terminalResize: async (_cols: number, _rows: number) => {},
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

  // Event listeners return no-op unsubscribers in mock mode
  onSerialOutput: async (_cb: (data: number[]) => void) => () => {},
  onVmStateChanged: async (_cb: (state: string) => void) => () => {},
  onTerminalSourceChanged: async (_cb: (source: string) => void) => () => {},
};
