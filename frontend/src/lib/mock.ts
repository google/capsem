// Mock data for browser-only dev mode (no Tauri backend).
// Active when window.__TAURI_INTERNALS__ is absent.
//
// Settings + MCP data imported from generated file (built from TOML configs + Rust tool defs).
// Only non-generated mock data (VM state, logs, session info, callMcpTool responses) lives here.
import type {
  ConfigIssue,
  HostConfig,
  KeyValidation,
  LogEntry,
  LogSessionInfo,
  McpPolicyInfo,
  McpServerInfo,
  McpToolInfo,
  QueryResult,
  SessionInfo,
  VmStateResponse,
  GuestConfigResponse,
  NetworkPolicyResponse,
} from './types';

export const isMock = typeof window !== 'undefined' && !(window as any).__TAURI_INTERNALS__;

// Callback stored from onVmStateChanged for download-complete transition.
let mockVmStateCallback: ((payload: { state: string; trigger?: string; message?: string }) => void) | null = null;

// ---------------------------------------------------------------------------
// Generated data (settings + MCP from TOML configs + Rust tool defs)
// ---------------------------------------------------------------------------

import {
  mockSettings, recomputeEnabled, buildMockTree,
  MOCK_MCP_SERVERS as _GEN_SERVERS,
  MOCK_MCP_TOOLS as _GEN_TOOLS,
  MOCK_MCP_POLICY as _GEN_POLICY,
} from './mock-settings.generated';

// Mutable copies so mock API can modify them at runtime.
let MOCK_MCP_SERVERS = _GEN_SERVERS;
let MOCK_MCP_TOOLS = _GEN_TOOLS;
let MOCK_MCP_POLICY: McpPolicyInfo = { ..._GEN_POLICY, tool_permissions: { ..._GEN_POLICY.tool_permissions } };

/** Compute lint issues dynamically from current mock settings. */
function computeMockLint(): ConfigIssue[] {
  const issues: ConfigIssue[] = [];
  for (const s of mockSettings) {
    if (s.setting_type === 'apikey' && s.enabled_by) {
      const toggle = mockSettings.find(t => t.id === s.enabled_by);
      if (toggle?.effective_value === true && !String(s.effective_value).trim()) {
        issues.push({
          id: s.id,
          severity: 'warning',
          message: `${s.name} not set`,
          docs_url: s.metadata.docs_url ?? null,
        });
      }
    }
  }
  return issues;
}

// Set initial enabled flags from the declared settings.
recomputeEnabled();

// ---------------------------------------------------------------------------
// VM state mock (non-generated)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Exported mock API
// ---------------------------------------------------------------------------

export const mockApi = {
  vmStatus: async () => 'running',
  serialInput: async (_input: string) => {},
  terminalResize: async (_cols: number, _rows: number) => {},
  getGuestConfig: async (): Promise<GuestConfigResponse> => ({ env: { TERM: 'xterm-256color', HOME: '/root' } }),
  getNetworkPolicy: async (): Promise<NetworkPolicyResponse> => ({
    allow: [
      'github.com', '*.github.com', '*.githubusercontent.com',
      'deb.debian.org', 'security.debian.org',
      'registry.npmjs.org', '*.npmjs.org',
      'pypi.org', 'files.pythonhosted.org',
      'crates.io', 'static.crates.io',
      '*.googleapis.com',
      'www.google.com', 'google.com',
      'elie.net', '*.elie.net', 'ash-speed.hetzner.com',
    ],
    block: [
      '*.anthropic.com', '*.claude.com',
      '*.openai.com',
      'www.bing.com', 'bing.com',
      'duckduckgo.com', '*.duckduckgo.com',
    ],
    default_action: 'deny',
    corp_managed: false,
    conflicts: [],
  }),
  setGuestEnv: async (_key: string, _value: string) => {},
  removeGuestEnv: async (_key: string) => {},
  getSettings: async () => mockSettings.map(s => ({ ...s })),
  getSettingsTree: async () => buildMockTree(),
  lintConfig: async () => computeMockLint(),
  listPresets: async () => [
    {
      id: 'medium',
      name: 'Medium Security',
      description: 'Allows read-only web access (GET/HEAD) and all search engines. Blocks write requests. MCP tools run without confirmation.',
      settings: {
        'security.web.allow_read': true,
        'security.web.allow_write': false,
        'security.services.search.google.allow': true,
        'security.services.search.bing.allow': true,
        'security.services.search.duckduckgo.allow': true,
      },
      mcp: { default_tool_permission: 'allow' },
    },
    {
      id: 'high',
      name: 'High Security',
      description: 'Blocks all web access by default. Only Google search is allowed. MCP tools require confirmation before running.',
      settings: {
        'security.web.allow_read': false,
        'security.web.allow_write': false,
        'security.services.search.google.allow': true,
        'security.services.search.bing.allow': false,
        'security.services.search.duckduckgo.allow': false,
      },
      mcp: { default_tool_permission: 'warn' },
    },
  ],
  applyPreset: async (id: string) => {
    const presets: Record<string, Record<string, any>> = {
      medium: {
        'security.web.allow_read': true,
        'security.web.allow_write': false,
        'security.services.search.google.allow': true,
        'security.services.search.bing.allow': true,
        'security.services.search.duckduckgo.allow': true,
      },
      high: {
        'security.web.allow_read': false,
        'security.web.allow_write': false,
        'security.services.search.google.allow': true,
        'security.services.search.bing.allow': false,
        'security.services.search.duckduckgo.allow': false,
      },
    };
    const settings = presets[id];
    if (!settings) return [];
    for (const [key, value] of Object.entries(settings)) {
      const s = mockSettings.find(s => s.id === key);
      if (s && !s.corp_locked) {
        s.effective_value = value;
        s.source = 'user';
        s.modified = new Date().toISOString();
      }
    }
    recomputeEnabled();
    return [];
  },
  updateSetting: async (id: string, value: any) => {
    const s = mockSettings.find(s => s.id === id);
    if (!s || s.corp_locked) return;
    s.effective_value = value;
    s.source = 'user';
    s.modified = new Date().toISOString();
    recomputeEnabled();
  },
  loadSettings: async () => ({
    tree: buildMockTree(),
    issues: computeMockLint(),
    presets: await mockApi.listPresets(),
  }),
  saveSettings: async (changes: Record<string, any>) => {
    for (const [id, value] of Object.entries(changes)) {
      const s = mockSettings.find(s => s.id === id);
      if (!s || s.corp_locked) continue;
      s.effective_value = value;
      s.source = 'user' as const;
      s.modified = new Date().toISOString();
    }
    recomputeEnabled();
    return {
      tree: buildMockTree(),
      issues: computeMockLint(),
      presets: await mockApi.listPresets(),
    };
  },
  getVmState: async () => MOCK_VM_STATE,
  getMcpServers: async (): Promise<McpServerInfo[]> => MOCK_MCP_SERVERS.map(s => ({ ...s })),
  getMcpTools: async (): Promise<McpToolInfo[]> => MOCK_MCP_TOOLS.map(t => ({ ...t })),
  getMcpPolicy: async (): Promise<McpPolicyInfo> => ({ ...MOCK_MCP_POLICY, tool_permissions: { ...MOCK_MCP_POLICY.tool_permissions } }),
  setMcpServerEnabled: async (name: string, enabled: boolean) => {
    const s = MOCK_MCP_SERVERS.find(s => s.name === name);
    if (s) { s.enabled = enabled; s.running = enabled; }
  },
  addMcpServer: async (name: string, url: string, _headers: Record<string, string>, bearerToken: string | null) => {
    MOCK_MCP_SERVERS = [...MOCK_MCP_SERVERS, { name, url, has_bearer_token: !!bearerToken, custom_header_count: Object.keys(_headers).length, source: 'manual', enabled: true, running: true, tool_count: 0, unsupported_stdio: false }];
  },
  removeMcpServer: async (name: string) => {
    MOCK_MCP_SERVERS = MOCK_MCP_SERVERS.filter(s => s.name !== name);
    MOCK_MCP_TOOLS = MOCK_MCP_TOOLS.filter(t => t.server_name !== name);
  },
  setMcpGlobalPolicy: async (policy: string) => {
    MOCK_MCP_POLICY = { ...MOCK_MCP_POLICY, global_policy: policy };
  },
  setMcpDefaultPermission: async (permission: string) => {
    MOCK_MCP_POLICY = { ...MOCK_MCP_POLICY, default_tool_permission: permission };
  },
  setMcpToolPermission: async (tool: string, permission: string) => {
    MOCK_MCP_POLICY = { ...MOCK_MCP_POLICY, tool_permissions: { ...MOCK_MCP_POLICY.tool_permissions, [tool]: permission } };
  },
  approveMcpTool: async (tool: string) => {
    const t = MOCK_MCP_TOOLS.find(t => t.namespaced_name === tool);
    if (t) { t.approved = true; t.pin_changed = false; }
  },
  refreshMcpTools: async (_server?: string) => {},
  getSessionInfo: async (): Promise<SessionInfo> => ({
    session_id: '20260225-143052-a7f3',
    mode: 'gui',
    uptime_ms: 45000,
    scratch_disk_size_gb: 8,
    ram_bytes: 512 * 1024 * 1024,
    total_requests: 23,
    allowed_requests: 17,
    denied_requests: 6,
    error_requests: 0,
    bytes_sent: 45000,
    bytes_received: 128000,
    model_call_count: 12,
    total_input_tokens: 45200,
    total_output_tokens: 12800,
    total_usage_details: {},
    total_tool_calls: 67,
    total_estimated_cost_usd: 0.42,
  }),

  detectHostConfig: async (): Promise<HostConfig> => ({
    git_name: 'Alice Example',
    git_email: 'alice@example.com',
    ssh_public_key: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample alice@macbook',
    anthropic_api_key: 'sk-ant-api03-detected...',
    google_api_key: null,
    openai_api_key: null,
    github_token: 'ghp_detected1234567890',
    claude_oauth_credentials: '{"claudeAiOauth":{"accessToken":"sk-ant-oat01-mock","refreshToken":"sk-ant-ort01-mock","expiresAt":9999999999}}',
    google_adc: null,
  }),

  validateApiKey: async (provider: string, key: string): Promise<KeyValidation> => {
    await new Promise((r) => setTimeout(r, 800));
    const prefixes: Record<string, string> = {
      anthropic: 'sk-ant-',
      openai: 'sk-',
      google: 'AIza',
      github: 'ghp_',
    };
    const prefix = prefixes[provider];
    const valid = !!prefix && key.startsWith(prefix) && key.length > prefix.length + 4;
    return {
      valid,
      message: valid ? 'Valid' : 'Invalid API key',
    };
  },

  // Event listeners return no-op unsubscribers in mock mode
  onSerialOutput: async (_cb: (data: number[]) => void) => () => {},
  onVmStateChanged: async (cb: (payload: { state: string; trigger?: string; message?: string }) => void) => {
    mockVmStateCallback = cb;
    return () => { mockVmStateCallback = null; };
  },
  onTerminalSourceChanged: async (_cb: (source: string) => void) => () => {},
  onDownloadProgress: async (_cb: (progress: any) => void) => {
    return () => {};
  },
  checkForAppUpdate: async () => null,

  onLogEvent: async (cb: (entry: LogEntry) => void) => {
    const mockEntries: LogEntry[] = [
      { timestamp: '2026-03-17T10:05:30.100Z', level: 'INFO', target: 'capsem::vm::boot', message: 'resolving assets' },
      { timestamp: '2026-03-17T10:05:30.200Z', level: 'INFO', target: 'capsem::vm::boot', message: 'creating VM' },
      { timestamp: '2026-03-17T10:05:31.400Z', level: 'INFO', target: 'capsem::vm::boot', message: 'kernel loaded' },
      { timestamp: '2026-03-17T10:05:32.700Z', level: 'INFO', target: 'capsem::vm::vsock', message: 'connected port 5001' },
      { timestamp: '2026-03-17T10:05:32.800Z', level: 'WARN', target: 'capsem::mcp::init', message: 'MCP server timeout, retrying' },
      { timestamp: '2026-03-17T10:05:33.100Z', level: 'INFO', target: 'capsem::mcp::init', message: 'MCP gateway initialized' },
      { timestamp: '2026-03-17T10:05:33.200Z', level: 'INFO', target: 'capsem::vm::boot', message: 'VM running' },
    ];
    let i = 0;
    const iv = setInterval(() => {
      if (i < mockEntries.length) {
        cb(mockEntries[i]);
        i++;
      } else {
        clearInterval(iv);
      }
    }, 500);
    return () => clearInterval(iv);
  },

  loadSessionLog: async (_sessionId: string): Promise<LogEntry[]> => [
    { timestamp: '2026-03-16T14:20:01.000Z', level: 'INFO', target: 'capsem::vm::boot', message: 'resolving assets' },
    { timestamp: '2026-03-16T14:20:01.500Z', level: 'INFO', target: 'capsem::vm::boot', message: 'creating VM' },
    { timestamp: '2026-03-16T14:20:03.000Z', level: 'INFO', target: 'capsem::vm::boot', message: 'kernel loaded' },
    { timestamp: '2026-03-16T14:20:04.200Z', level: 'INFO', target: 'capsem::vm::vsock', message: 'connected port 5001' },
    { timestamp: '2026-03-16T14:20:04.500Z', level: 'INFO', target: 'capsem::vm::boot', message: 'VM running' },
    { timestamp: '2026-03-16T14:35:00.000Z', level: 'ERROR', target: 'capsem::net::mitm', message: 'codesign verification failed: signature invalid' },
  ],

  listLogSessions: async (): Promise<LogSessionInfo[]> => [
    { session_id: '20260317-100530-a1b2', entry_count: 7 },
    { session_id: '20260316-142001-c3d4', entry_count: 6 },
    { session_id: '20260315-091500-e5f6', entry_count: 12 },
  ],

  callMcpTool: async (_tool: string, _args: Record<string, unknown> = {}): Promise<unknown> => {
    return { content: [{ text: '{}' }] };
  },
};

// ---------------------------------------------------------------------------
// sql.js-backed fixture queries for mock mode
// ---------------------------------------------------------------------------

import initSqlJs, { type Database } from 'sql.js';

let fixtureDb: Database | null = null;
let fixtureLoading: Promise<Database> | null = null;

async function getFixtureDb(): Promise<Database> {
  if (fixtureDb) return fixtureDb;
  if (fixtureLoading) return fixtureLoading;
  fixtureLoading = (async () => {
    const SQL = await initSqlJs({
      locateFile: (file: string) => `/node_modules/sql.js/dist/${file}`,
    });
    const resp = await fetch('/fixtures/test.db');
    const buf = await resp.arrayBuffer();
    fixtureDb = new SQL.Database(new Uint8Array(buf));
    return fixtureDb;
  })();
  return fixtureLoading;
}

function runQuery(db: Database, sql: string, params?: unknown[]): QueryResult {
  const stmt = db.prepare(sql);
  if (params && params.length > 0) {
    stmt.bind(params as any);
  }
  const columns: string[] = stmt.getColumnNames();
  const rows: unknown[][] = [];
  while (stmt.step()) {
    rows.push(stmt.get());
  }
  stmt.free();
  return { columns, rows };
}

/** Run a query against the fixture session DB (test.db). */
export async function queryFixture(sql: string, params?: unknown[]): Promise<QueryResult> {
  const db = await getFixtureDb();
  return runQuery(db, sql, params);
}

/** Run a query against fixture -- same DB in mock mode (no separate main.db). */
export async function queryFixtureMain(sql: string, params?: unknown[]): Promise<QueryResult> {
  const db = await getFixtureDb();
  return runQuery(db, sql, params);
}
