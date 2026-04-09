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

