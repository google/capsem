export interface MockVM {
  id: string;
  name: string;
  status: 'running' | 'stopped' | 'booting' | 'error';
  ram: number;
  cpus: number;
  persistent: boolean;
  uptime: string;
}

export const mockVMs: MockVM[] = [
  { id: 'vm-1', name: 'dev-sandbox', status: 'running', ram: 2048, cpus: 2, persistent: true, uptime: '2h 14m' },
  { id: 'vm-2', name: 'ci-runner', status: 'running', ram: 4096, cpus: 4, persistent: false, uptime: '45m' },
  { id: 'vm-3', name: 'pentest-box', status: 'stopped', ram: 1024, cpus: 1, persistent: true, uptime: '--' },
  { id: 'vm-4', name: 'ml-training', status: 'error', ram: 8192, cpus: 8, persistent: false, uptime: '--' },
  { id: 'vm-5', name: 'web-scraper', status: 'booting', ram: 2048, cpus: 2, persistent: false, uptime: '--' },
];

export function getVM(id: string): MockVM | undefined {
  return mockVMs.find(vm => vm.id === id);
}

// ---------------------------------------------------------------------------
// Stats mock data
// ---------------------------------------------------------------------------

export interface MockModelStats {
  provider: string;
  model: string;
  inputTokens: number;
  outputTokens: number;
  cacheTokens: number;
  estimatedCostUsd: number;
  callCount: number;
}

export interface MockToolCall {
  id: string;
  tool: string;
  server: string;
  args: string;
  result: string;
  durationMs: number;
  timestamp: string;
}

export interface MockNetworkEvent {
  id: string;
  method: string;
  url: string;
  status: number;
  decision: 'allowed' | 'denied';
  durationMs: number;
  bytesSent: number;
  bytesReceived: number;
  timestamp: string;
}

export interface MockFileEvent {
  id: string;
  path: string;
  operation: 'created' | 'modified' | 'deleted';
  sizeBytes: number | null;
  timestamp: string;
}

export const mockModelStats: MockModelStats[] = [
  { provider: 'Anthropic', model: 'claude-sonnet-4-6', inputTokens: 32400, outputTokens: 8900, cacheTokens: 12000, estimatedCostUsd: 0.31, callCount: 8 },
  { provider: 'OpenAI', model: 'gpt-4o', inputTokens: 12800, outputTokens: 3900, cacheTokens: 0, estimatedCostUsd: 0.11, callCount: 4 },
];

export const mockToolCalls: MockToolCall[] = [
  { id: 'tc-1', tool: 'read_file', server: 'filesystem', args: '{"path": "/src/main.rs"}', result: '{"content": "fn main() { ... }"}', durationMs: 45, timestamp: '2026-04-09T10:05:30.100Z' },
  { id: 'tc-2', tool: 'write_file', server: 'filesystem', args: '{"path": "/src/lib.rs", "content": "..."}', result: '{"success": true}', durationMs: 32, timestamp: '2026-04-09T10:05:31.200Z' },
  { id: 'tc-3', tool: 'bash', server: 'system', args: '{"command": "cargo build"}', result: '{"exit_code": 0, "stdout": "Compiling..."}', durationMs: 4200, timestamp: '2026-04-09T10:05:35.000Z' },
  { id: 'tc-4', tool: 'search_files', server: 'filesystem', args: '{"pattern": "TODO", "path": "/src"}', result: '{"matches": ["/src/main.rs:12"]}', durationMs: 120, timestamp: '2026-04-09T10:06:00.000Z' },
  { id: 'tc-5', tool: 'list_dir', server: 'filesystem', args: '{"path": "/src"}', result: '{"entries": ["main.rs", "lib.rs", "utils/"]}', durationMs: 18, timestamp: '2026-04-09T10:06:10.000Z' },
  { id: 'tc-6', tool: 'bash', server: 'system', args: '{"command": "cargo test"}', result: '{"exit_code": 0, "stdout": "test result: ok. 12 passed"}', durationMs: 8500, timestamp: '2026-04-09T10:07:00.000Z' },
];

export const mockNetworkEvents: MockNetworkEvent[] = [
  { id: 'ne-1', method: 'POST', url: 'https://api.anthropic.com/v1/messages', status: 200, decision: 'allowed', durationMs: 1200, bytesSent: 4500, bytesReceived: 12000, timestamp: '2026-04-09T10:05:30.000Z' },
  { id: 'ne-2', method: 'GET', url: 'https://registry.npmjs.org/express', status: 200, decision: 'allowed', durationMs: 340, bytesSent: 200, bytesReceived: 8500, timestamp: '2026-04-09T10:05:32.000Z' },
  { id: 'ne-3', method: 'POST', url: 'https://api.openai.com/v1/chat/completions', status: 200, decision: 'allowed', durationMs: 2100, bytesSent: 3200, bytesReceived: 9800, timestamp: '2026-04-09T10:05:35.000Z' },
  { id: 'ne-4', method: 'GET', url: 'https://evil.example.com/exfiltrate', status: 0, decision: 'denied', durationMs: 0, bytesSent: 0, bytesReceived: 0, timestamp: '2026-04-09T10:05:36.000Z' },
  { id: 'ne-5', method: 'GET', url: 'https://github.com/anthropics/capsem', status: 200, decision: 'allowed', durationMs: 450, bytesSent: 300, bytesReceived: 45000, timestamp: '2026-04-09T10:06:00.000Z' },
  { id: 'ne-6', method: 'POST', url: 'https://bing.com/search', status: 0, decision: 'denied', durationMs: 0, bytesSent: 0, bytesReceived: 0, timestamp: '2026-04-09T10:06:05.000Z' },
];

export const mockFileEvents: MockFileEvent[] = [
  { id: 'fe-1', path: '/workspace/src/main.rs', operation: 'modified', sizeBytes: 2048, timestamp: '2026-04-09T10:05:30.000Z' },
  { id: 'fe-2', path: '/workspace/src/utils/helpers.rs', operation: 'created', sizeBytes: 512, timestamp: '2026-04-09T10:05:31.000Z' },
  { id: 'fe-3', path: '/workspace/src/old_module.rs', operation: 'deleted', sizeBytes: null, timestamp: '2026-04-09T10:05:32.000Z' },
  { id: 'fe-4', path: '/workspace/Cargo.toml', operation: 'modified', sizeBytes: 1024, timestamp: '2026-04-09T10:05:35.000Z' },
  { id: 'fe-5', path: '/workspace/src/lib.rs', operation: 'modified', sizeBytes: 4096, timestamp: '2026-04-09T10:06:00.000Z' },
  { id: 'fe-6', path: '/workspace/tests/integration.rs', operation: 'created', sizeBytes: 768, timestamp: '2026-04-09T10:06:10.000Z' },
];

// ---------------------------------------------------------------------------
// Log mock data
// ---------------------------------------------------------------------------

export interface MockLogEntry {
  id: string;
  timestamp: string;
  level: 'info' | 'warn' | 'error';
  source: string;
  message: string;
}

// ---------------------------------------------------------------------------
// File tree mock data
// ---------------------------------------------------------------------------

export interface MockFileNode {
  name: string;
  type: 'file' | 'directory';
  path: string;
  children?: MockFileNode[];
  content?: string;
  sizeBytes?: number;
}

export const mockFileTree: MockFileNode[] = [
  {
    name: 'src', type: 'directory', path: '/workspace/src',
    children: [
      {
        name: 'main.rs', type: 'file', path: '/workspace/src/main.rs', sizeBytes: 482,
        content: 'use std::io;\n\nmod utils;\n\nfn main() {\n    println!("Capsem sandbox ready");\n    let config = utils::load_config();\n    if let Err(e) = run(config) {\n        eprintln!("Fatal: {e}");\n        std::process::exit(1);\n    }\n}\n\nfn run(config: utils::Config) -> io::Result<()> {\n    // Main event loop\n    loop {\n        std::thread::sleep(std::time::Duration::from_secs(1));\n    }\n}',
      },
      {
        name: 'lib.rs', type: 'file', path: '/workspace/src/lib.rs', sizeBytes: 1024,
        content: '//! Core library for the sandbox agent.\n\npub mod executor;\npub mod sandbox;\n\npub use executor::Executor;\npub use sandbox::Sandbox;\n\n/// Version of the guest agent protocol.\npub const PROTOCOL_VERSION: u32 = 3;',
      },
      {
        name: 'utils', type: 'directory', path: '/workspace/src/utils',
        children: [
          {
            name: 'mod.rs', type: 'file', path: '/workspace/src/utils/mod.rs', sizeBytes: 256,
            content: 'mod config;\nmod logging;\n\npub use config::{Config, load_config};\npub use logging::init_logging;',
          },
          {
            name: 'config.rs', type: 'file', path: '/workspace/src/utils/config.rs', sizeBytes: 640,
            content: 'use std::path::PathBuf;\n\n#[derive(Debug, Clone)]\npub struct Config {\n    pub workspace: PathBuf,\n    pub ram_mb: u64,\n    pub cpus: u32,\n}\n\npub fn load_config() -> Config {\n    Config {\n        workspace: PathBuf::from("/workspace"),\n        ram_mb: 2048,\n        cpus: 2,\n    }\n}',
          },
          {
            name: 'logging.rs', type: 'file', path: '/workspace/src/utils/logging.rs', sizeBytes: 320,
            content: 'use tracing_subscriber::EnvFilter;\n\npub fn init_logging() {\n    tracing_subscriber::fmt()\n        .with_env_filter(EnvFilter::from_default_env())\n        .init();\n}',
          },
        ],
      },
    ],
  },
  {
    name: 'tests', type: 'directory', path: '/workspace/tests',
    children: [
      {
        name: 'integration.rs', type: 'file', path: '/workspace/tests/integration.rs', sizeBytes: 768,
        content: '#[test]\nfn test_sandbox_boots() {\n    let sandbox = Sandbox::new_ephemeral();\n    assert!(sandbox.boot().is_ok());\n    assert_eq!(sandbox.status(), Status::Running);\n}\n\n#[test]\nfn test_exec_command() {\n    let sandbox = Sandbox::new_ephemeral();\n    sandbox.boot().unwrap();\n    let result = sandbox.exec("echo hello").unwrap();\n    assert_eq!(result.stdout.trim(), "hello");\n    assert_eq!(result.exit_code, 0);\n}',
      },
    ],
  },
  {
    name: 'Cargo.toml', type: 'file', path: '/workspace/Cargo.toml', sizeBytes: 380,
    content: '[package]\nname = "capsem-guest"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\ntracing = "0.1"\ntracing-subscriber = { version = "0.3", features = ["env-filter"] }\nserde = { version = "1", features = ["derive"] }\nserde_json = "1"\ntokio = { version = "1", features = ["full"] }',
  },
  {
    name: 'README.md', type: 'file', path: '/workspace/README.md', sizeBytes: 210,
    content: '# capsem-guest\n\nGuest agent for the Capsem sandbox.\n\n## Build\n\n```bash\ncargo build --release\n```\n\n## Test\n\n```bash\ncargo test\n```',
  },
];

export function findFileNode(tree: MockFileNode[], path: string): MockFileNode | undefined {
  for (const node of tree) {
    if (node.path === path) return node;
    if (node.children) {
      const found = findFileNode(node.children, path);
      if (found) return found;
    }
  }
  return undefined;
}

// ---------------------------------------------------------------------------
// Inspector mock data
// ---------------------------------------------------------------------------

export interface MockQueryResult {
  columns: string[];
  rows: Record<string, string | number | null>[];
}

export interface MockPresetQuery {
  label: string;
  sql: string;
}

export const mockPresetQueries: MockPresetQuery[] = [
  { label: 'Recent events', sql: 'SELECT timestamp, event_type, summary FROM event_log ORDER BY timestamp DESC LIMIT 20' },
  { label: 'HTTP requests', sql: 'SELECT method, url, status_code, decision, duration_ms FROM http_requests ORDER BY timestamp DESC LIMIT 20' },
  { label: 'Tool calls', sql: 'SELECT tool_name, server, duration_ms, timestamp FROM tool_calls ORDER BY timestamp DESC LIMIT 20' },
  { label: 'Model calls', sql: 'SELECT provider, model, input_tokens, output_tokens, estimated_cost_usd FROM model_calls ORDER BY timestamp DESC' },
  { label: 'File events', sql: 'SELECT path, operation, size_bytes, timestamp FROM file_events ORDER BY timestamp DESC LIMIT 20' },
];

export const mockQueryResults: Record<string, MockQueryResult> = {
  'Recent events': {
    columns: ['timestamp', 'event_type', 'summary'],
    rows: [
      { timestamp: '2026-04-09 10:08:00', event_type: 'snapshot', summary: 'Auto-snapshot triggered' },
      { timestamp: '2026-04-09 10:07:30', event_type: 'network', summary: 'Certificate verification failed' },
      { timestamp: '2026-04-09 10:07:00', event_type: 'tool_call', summary: 'bash: cargo test (8.5s)' },
      { timestamp: '2026-04-09 10:06:10', event_type: 'file', summary: '6 file events in /workspace/src/' },
      { timestamp: '2026-04-09 10:06:05', event_type: 'network', summary: 'Denied: bing.com' },
      { timestamp: '2026-04-09 10:06:00', event_type: 'network', summary: 'TLS intercept: github.com' },
      { timestamp: '2026-04-09 10:05:36', event_type: 'network', summary: 'Denied: evil.example.com' },
      { timestamp: '2026-04-09 10:05:35', event_type: 'tool_call', summary: 'bash: cargo build (4.2s)' },
    ],
  },
  'HTTP requests': {
    columns: ['method', 'url', 'status_code', 'decision', 'duration_ms'],
    rows: [
      { method: 'POST', url: 'https://api.anthropic.com/v1/messages', status_code: 200, decision: 'allowed', duration_ms: 1200 },
      { method: 'GET', url: 'https://registry.npmjs.org/express', status_code: 200, decision: 'allowed', duration_ms: 340 },
      { method: 'POST', url: 'https://api.openai.com/v1/chat/completions', status_code: 200, decision: 'allowed', duration_ms: 2100 },
      { method: 'GET', url: 'https://evil.example.com/exfiltrate', status_code: null, decision: 'denied', duration_ms: 0 },
      { method: 'GET', url: 'https://github.com/anthropics/capsem', status_code: 200, decision: 'allowed', duration_ms: 450 },
    ],
  },
  'Tool calls': {
    columns: ['tool_name', 'server', 'duration_ms', 'timestamp'],
    rows: [
      { tool_name: 'read_file', server: 'filesystem', duration_ms: 45, timestamp: '2026-04-09 10:05:30' },
      { tool_name: 'write_file', server: 'filesystem', duration_ms: 32, timestamp: '2026-04-09 10:05:31' },
      { tool_name: 'bash', server: 'system', duration_ms: 4200, timestamp: '2026-04-09 10:05:35' },
      { tool_name: 'search_files', server: 'filesystem', duration_ms: 120, timestamp: '2026-04-09 10:06:00' },
      { tool_name: 'list_dir', server: 'filesystem', duration_ms: 18, timestamp: '2026-04-09 10:06:10' },
      { tool_name: 'bash', server: 'system', duration_ms: 8500, timestamp: '2026-04-09 10:07:00' },
    ],
  },
  'Model calls': {
    columns: ['provider', 'model', 'input_tokens', 'output_tokens', 'estimated_cost_usd'],
    rows: [
      { provider: 'Anthropic', model: 'claude-sonnet-4-6', input_tokens: 32400, output_tokens: 8900, estimated_cost_usd: 0.31 },
      { provider: 'OpenAI', model: 'gpt-4o', input_tokens: 12800, output_tokens: 3900, estimated_cost_usd: 0.11 },
    ],
  },
  'File events': {
    columns: ['path', 'operation', 'size_bytes', 'timestamp'],
    rows: [
      { path: '/workspace/src/main.rs', operation: 'modified', size_bytes: 2048, timestamp: '2026-04-09 10:05:30' },
      { path: '/workspace/src/utils/helpers.rs', operation: 'created', size_bytes: 512, timestamp: '2026-04-09 10:05:31' },
      { path: '/workspace/src/old_module.rs', operation: 'deleted', size_bytes: null, timestamp: '2026-04-09 10:05:32' },
      { path: '/workspace/Cargo.toml', operation: 'modified', size_bytes: 1024, timestamp: '2026-04-09 10:05:35' },
      { path: '/workspace/src/lib.rs', operation: 'modified', size_bytes: 4096, timestamp: '2026-04-09 10:06:00' },
      { path: '/workspace/tests/integration.rs', operation: 'created', size_bytes: 768, timestamp: '2026-04-09 10:06:10' },
    ],
  },
};

/**
 * Validate that a SQL query is a SELECT statement (not INSERT/UPDATE/DELETE/DROP/etc).
 * Returns null if valid, or an error message if rejected.
 */
export function validateSelectOnly(sql: string): string | null {
  const trimmed = sql.trim();
  if (!trimmed) return 'Query is empty';

  // Strip leading comments (-- and /* */)
  const stripped = trimmed
    .replace(/--[^\n]*/g, '')
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .trim();

  if (!stripped) return 'Query is empty (only comments)';

  // Must start with SELECT (case-insensitive)
  if (!/^SELECT\b/i.test(stripped)) {
    return 'Only SELECT queries are allowed';
  }

  // Reject dangerous keywords anywhere in the query
  const dangerous = /\b(INSERT|UPDATE|DELETE|DROP|ALTER|CREATE|TRUNCATE|REPLACE|ATTACH|DETACH|PRAGMA)\b/i;
  if (dangerous.test(stripped)) {
    return 'Query contains forbidden keyword';
  }

  return null;
}

/**
 * Execute a mock query by matching against preset labels or returning a generic result.
 */
export function executeMockQuery(sql: string): MockQueryResult {
  // Try to match against presets
  for (const preset of mockPresetQueries) {
    if (sql.trim() === preset.sql.trim()) {
      return mockQueryResults[preset.label];
    }
  }

  // Generic fallback for any SELECT
  return {
    columns: ['result'],
    rows: [{ result: '(mock mode -- preset queries available via dropdown)' }],
  };
}

// ---------------------------------------------------------------------------
// Log mock data
// ---------------------------------------------------------------------------

export const mockLogEntries: MockLogEntry[] = [
  { id: 'log-01', timestamp: '2026-04-09T10:05:30.100Z', level: 'info', source: 'vm::boot', message: 'Resolving assets for aarch64' },
  { id: 'log-02', timestamp: '2026-04-09T10:05:30.200Z', level: 'info', source: 'vm::boot', message: 'Creating VM with 2048 MB RAM, 2 CPUs' },
  { id: 'log-03', timestamp: '2026-04-09T10:05:31.400Z', level: 'info', source: 'vm::boot', message: 'Kernel loaded (6.6.127-capsem)' },
  { id: 'log-04', timestamp: '2026-04-09T10:05:32.700Z', level: 'info', source: 'vm::vsock', message: 'Connected control channel on port 5000' },
  { id: 'log-05', timestamp: '2026-04-09T10:05:32.800Z', level: 'warn', source: 'mcp::init', message: 'MCP server timeout on first attempt, retrying (1/3)' },
  { id: 'log-06', timestamp: '2026-04-09T10:05:33.100Z', level: 'info', source: 'mcp::init', message: 'MCP gateway initialized with 3 servers' },
  { id: 'log-07', timestamp: '2026-04-09T10:05:33.200Z', level: 'info', source: 'vm::boot', message: 'VM running -- boot completed in 3.1s' },
  { id: 'log-08', timestamp: '2026-04-09T10:05:35.000Z', level: 'info', source: 'net::mitm', message: 'TLS intercept: api.anthropic.com (allowed)' },
  { id: 'log-09', timestamp: '2026-04-09T10:05:36.000Z', level: 'warn', source: 'net::policy', message: 'Denied request to evil.example.com (not in allowlist)' },
  { id: 'log-10', timestamp: '2026-04-09T10:06:00.000Z', level: 'info', source: 'net::mitm', message: 'TLS intercept: github.com (allowed)' },
  { id: 'log-11', timestamp: '2026-04-09T10:06:05.000Z', level: 'warn', source: 'net::policy', message: 'Denied request to bing.com (blocked by policy)' },
  { id: 'log-12', timestamp: '2026-04-09T10:06:10.000Z', level: 'info', source: 'fs::monitor', message: 'File watch: 6 events in /workspace/src/' },
  { id: 'log-13', timestamp: '2026-04-09T10:07:00.000Z', level: 'info', source: 'mcp::gateway', message: 'Tool call: bash (cargo test) -- 8.5s' },
  { id: 'log-14', timestamp: '2026-04-09T10:07:30.000Z', level: 'error', source: 'net::mitm', message: 'Certificate verification failed for malware.example.org' },
  { id: 'log-15', timestamp: '2026-04-09T10:08:00.000Z', level: 'info', source: 'snapshot', message: 'Auto-snapshot triggered (5 min interval)' },
];

