// Gateway API client. Token is module-scoped -- never in localStorage, DOM, logs, or URLs.

import type {
  StatusResponse,
  TokenResponse,
  HealthResponse,
  ProvisionRequest,
  ProvisionResponse,
  ExecResponse,
  InspectResponse,
  ReadFileResponse,
  ForkRequest,
  ForkResponse,
  StatsResponse,
} from './types/gateway';
import type {
  SettingsResponse,
  SecurityPreset,
  ConfigIssue,
} from './types/settings';
import type {
  DownloadProgress,
  McpServerInfo,
  McpToolInfo,
  McpPolicyInfo,
  VmStateResponse,
  FileListResponse,
  FileContentResult,
  FileUploadResponse,
} from './types';

// -- Module state (never exported directly) --

let _token: string | null = null;
let _connected = false;

// Derive gateway base URL:
// - When served by the gateway, window.location.origin IS the gateway.
// - In dev mode (Astro dev server on :5173), use the default gateway port.
// - In Tauri (tauri://), use the default gateway port.
const _GATEWAY_DEFAULT = 'http://127.0.0.1:19222';

function _detectBaseUrl(): string {
  if (typeof window === 'undefined') return _GATEWAY_DEFAULT;
  const { origin, port } = window.location;
  // Astro dev server -- API requests must go to the gateway, not the dev server
  if (port === '5173') return _GATEWAY_DEFAULT;
  // Same-origin localhost: we are served by the gateway
  if (origin.startsWith('http://127.0.0.1') || origin.startsWith('http://localhost')) {
    return origin;
  }
  return _GATEWAY_DEFAULT;
}

let _baseUrl = _detectBaseUrl();

// -- Public getters --

export function isConnected(): boolean {
  return _connected;
}

export function getBaseUrl(): string {
  return _baseUrl;
}

export type InitResult = {
  connected: boolean;
  reachable: boolean;
  version: string | null;
};

// -- Initialization --

export async function init(): Promise<InitResult> {
  console.log('[api] init() baseUrl=%s', _baseUrl);
  try {
    // Probe health first (unauthenticated)
    const healthResp = await fetch(`${_baseUrl}/health`);
    if (!healthResp.ok) {
      _connected = false;
      
      return { connected: false, reachable: false, version: null };
    }
    const health: HealthResponse = await healthResp.json();

    // Fetch token from gateway (unauthenticated, localhost-only)
    const tokenResp = await fetch(`${_baseUrl}/token`);
    if (!tokenResp.ok) {
      _connected = false;
      
      return { connected: false, reachable: true, version: health.version };
    }
    const tokenData: TokenResponse = await tokenResp.json();
    _token = tokenData.token;

    _connected = true;
    console.log('[api] init OK: connected, token acquired, version=%s', health.version);
    _connectEventWs();
    return { connected: true, reachable: true, version: health.version };
  } catch {
    _connected = false;
    _token = null;
    
    return { connected: false, reachable: false, version: null };
  }
}

export async function healthCheck(): Promise<boolean> {
  try {
    const resp = await fetch(`${_baseUrl}/health`);
    if (!resp.ok) return false;
    return true;
  } catch {
    _connected = false;
    
    return false;
  }
}

// -- HTTP helpers (private) --

class ApiError extends Error {
  constructor(
    public status: number,
    public body: string,
  ) {
    super(`API error ${status}: ${body}`);
    this.name = 'ApiError';
  }
}

async function _get(path: string): Promise<Response> {
  const resp = await fetch(`${_baseUrl}${path}`, {
    headers: { Authorization: `Bearer ${_token}` },
  });
  if (!resp.ok) {
    const body = await resp.text();
    throw new ApiError(resp.status, body);
  }
  return resp;
}

async function _post(path: string, body?: unknown): Promise<Response> {
  const resp = await fetch(`${_baseUrl}${path}`, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${_token}`,
      ...(body !== undefined ? { 'Content-Type': 'application/json' } : {}),
    },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  if (!resp.ok) {
    const text = await resp.text();
    throw new ApiError(resp.status, text);
  }
  return resp;
}

async function _delete(path: string): Promise<Response> {
  const resp = await fetch(`${_baseUrl}${path}`, {
    method: 'DELETE',
    headers: { Authorization: `Bearer ${_token}` },
  });
  if (!resp.ok) {
    const text = await resp.text();
    throw new ApiError(resp.status, text);
  }
  return resp;
}

// Helper: returns true if error is a network failure (gateway unreachable)
function isNetworkError(err: unknown): boolean {
  return !(err instanceof ApiError);
}

// -- Status --

export async function getStatus(): Promise<StatusResponse> {
  if (!_connected) {
    console.log('[api] getStatus() skipped: not connected');
    return emptyStatus();
  }
  try {
    const resp = await _get('/status');
    return await resp.json();
  } catch (err) {
    if (isNetworkError(err)) {
      _connected = false;
      return emptyStatus();
    }
    throw err;
  }
}

function emptyStatus(): StatusResponse {
  return {
    service: 'offline',
    gateway_version: '',
    vm_count: 0,
    vms: [],
    resource_summary: {
      total_ram_mb: 0,
      total_cpus: 0,
      running_count: 0,
      stopped_count: 0,
      suspended_count: 0,
    },
  };
}

// -- VM lifecycle --

export async function provisionVm(opts: ProvisionRequest): Promise<ProvisionResponse> {
  console.log('[api] provisionVm(%o) connected=%s', opts, _connected);
  const resp = await _post('/provision', opts);
  const result = await resp.json();
  console.log('[api] provisionVm result:', result);
  return result;
}

export async function runVm(opts: ProvisionRequest): Promise<ProvisionResponse> {
  const resp = await _post('/run', opts);
  return await resp.json();
}

export async function stopVm(id: string): Promise<void> {
  await _post(`/stop/${encodeURIComponent(id)}`);
}

export async function suspendVm(id: string): Promise<void> {
  await _post(`/suspend/${encodeURIComponent(id)}`);
}

export async function deleteVm(id: string): Promise<void> {
  await _delete(`/delete/${encodeURIComponent(id)}`);
}

export async function resumeVm(name: string): Promise<void> {
  await _post(`/resume/${encodeURIComponent(name)}`);
}

export async function persistVm(id: string, name: string): Promise<void> {
  await _post(`/persist/${encodeURIComponent(id)}`, { name });
}

export async function forkVm(id: string, opts: ForkRequest): Promise<ForkResponse> {
  const resp = await _post(`/fork/${encodeURIComponent(id)}`, opts);
  return await resp.json();
}

// -- VM inspection --

/** Raw log response from GET /logs/{id}. */
export interface RawLogsResponse {
  logs: string;
  serial_logs: string | null;
  process_logs: string | null;
}

export async function getVmLogs(id: string): Promise<RawLogsResponse> {
  if (!_connected) return { logs: '', serial_logs: null, process_logs: null };
  try {
    const resp = await _get(`/logs/${encodeURIComponent(id)}`);
    return await resp.json();
  } catch (err) {
    if (isNetworkError(err)) {
      _connected = false;
      return { logs: '', serial_logs: null, process_logs: null };
    }
    throw err;
  }
}

export async function getServiceLogs(): Promise<string> {
  if (!_connected) return '';
  try {
    const resp = await _get('/service-logs');
    return await resp.text();
  } catch (err) {
    if (isNetworkError(err)) {
      _connected = false;
      return '';
    }
    throw err;
  }
}

export async function execCommand(
  id: string,
  command: string,
  timeoutSecs?: number,
): Promise<ExecResponse> {
  const resp = await _post(`/exec/${encodeURIComponent(id)}`, {
    command,
    timeout_secs: timeoutSecs,
  });
  return await resp.json();
}

export async function inspectQuery(id: string, sql: string): Promise<InspectResponse> {
  if (!_connected) return { columns: [], rows: [] };
  try {
    const resp = await _post(`/inspect/${encodeURIComponent(id)}`, { sql });
    return await resp.json();
  } catch (err) {
    if (isNetworkError(err)) {
      _connected = false;
      return { columns: [], rows: [] };
    }
    throw err;
  }
}

export async function readFile(id: string, path: string): Promise<ReadFileResponse> {
  const resp = await _post(`/read_file/${encodeURIComponent(id)}`, { path });
  return await resp.json();
}

export async function writeFile(id: string, path: string, content: string): Promise<void> {
  await _post(`/write_file/${encodeURIComponent(id)}`, { path, content });
}

// -- Images --

export async function getImages(): Promise<{ images: { name: string }[] }> {
  const resp = await _get('/images');
  return await resp.json();
}

// -- Config --

export async function reloadConfig(): Promise<void> {
  await _post('/reload-config');
}

// -- Stats --

/** Fetch cross-session stats from main.db. */
export async function getStats(): Promise<StatsResponse> {
  if (!_connected) return emptyStats();
  try {
    const resp = await _get('/stats');
    return await resp.json();
  } catch (err) {
    if (isNetworkError(err)) {
      _connected = false;
      return emptyStats();
    }
    throw err;
  }
}

function emptyStats(): StatsResponse {
  return {
    global: {
      total_sessions: 0, total_input_tokens: 0, total_output_tokens: 0,
      total_estimated_cost: 0, total_tool_calls: 0, total_mcp_calls: 0,
      total_file_events: 0, total_requests: 0, total_allowed: 0, total_denied: 0,
    },
    sessions: [], top_providers: [], top_tools: [], top_mcp_tools: [],
  };
}

// -- Terminal --

export function getTerminalWsUrl(id: string): string {
  const wsBase = _baseUrl.replace(/^http/, 'ws');
  return `${wsBase}/terminal/${encodeURIComponent(id)}?token=${_token}`;
}

// Terminal WebSocket state (per-VM, lazy-connected).
let _termWs: WebSocket | null = null;
let _termBuffer: number[] = [];
let _termWaiter: ((data: number[]) => void) | null = null;
const _termSourceCallbacks: ((source: string) => void)[] = [];

/** Connect terminal WebSocket for a given VM. */
export function connectTerminal(id: string) {
  if (_termWs) {
    _termWs.close();
    _termWs = null;
  }
  _termBuffer = [];
  const url = getTerminalWsUrl(id);
  _termWs = new WebSocket(url);
  _termWs.binaryType = 'arraybuffer';
  _termWs.onopen = () => {
    for (const cb of _termSourceCallbacks) cb('websocket');
  };
  _termWs.onmessage = (ev) => {
    const data = Array.from(new Uint8Array(ev.data as ArrayBuffer));
    if (_termWaiter) {
      const w = _termWaiter;
      _termWaiter = null;
      w(data);
    } else {
      _termBuffer.push(...data);
    }
  };
  _termWs.onclose = () => {
    _termWs = null;
  };
}

/** Send input data to the terminal. */
export async function serialInput(data: string): Promise<void> {
  if (_termWs?.readyState === WebSocket.OPEN) {
    _termWs.send(new TextEncoder().encode(data));
  }
}

/** Poll for terminal output (returns buffered data or waits for next message). */
export async function terminalPoll(): Promise<number[]> {
  if (_termBuffer.length > 0) {
    const data = _termBuffer;
    _termBuffer = [];
    return data;
  }
  if (!_termWs || _termWs.readyState !== WebSocket.OPEN) {
    throw new Error('terminal closed');
  }
  return new Promise((resolve, reject) => {
    _termWaiter = resolve;
    // Reject if WebSocket closes while waiting.
    const ws = _termWs;
    const onClose = () => {
      if (_termWaiter === resolve) {
        _termWaiter = null;
        reject(new Error('terminal closed'));
      }
    };
    ws?.addEventListener('close', onClose, { once: true });
  });
}

/** Send a resize event to the terminal. */
export async function terminalResize(cols: number, rows: number): Promise<void> {
  if (_termWs?.readyState === WebSocket.OPEN) {
    _termWs.send(JSON.stringify({ type: 'resize', cols, rows }));
  }
}

/** Register a callback for terminal source changes (e.g., WebSocket connects). */
export async function onTerminalSourceChanged(cb: (source: string) => void): Promise<() => void> {
  _termSourceCallbacks.push(cb);
  return () => {
    const i = _termSourceCallbacks.indexOf(cb);
    if (i >= 0) _termSourceCallbacks.splice(i, 1);
  };
}

// -- VM state --

/** Get the current VM state string. Returns 'not created' in mock mode. */
export async function vmStatus(): Promise<string> {
  if (!_connected) return 'not created';
  try {
    const status = await getStatus();
    const running = status.vms.find(v => v.status.toLowerCase() === 'running');
    if (running) return running.status.toLowerCase();
    if (status.vms.length > 0) return status.vms[0].status.toLowerCase();
    return 'not created';
  } catch {
    return 'not created';
  }
}

/** Get VM state with transition history. */
export async function getVmState(id?: string): Promise<VmStateResponse> {
  if (!_connected) return { state: 'not created', elapsed_ms: 0, history: [] };
  try {
    const path = id ? `/info/${encodeURIComponent(id)}` : '/status';
    const resp = await _get(path);
    const data = await resp.json();
    // /info/{id} returns full sandbox info; extract state + history.
    if (id) {
      return {
        state: data.status ?? 'not created',
        elapsed_ms: data.elapsed_ms ?? 0,
        history: data.history ?? [],
      };
    }
    // /status: synthesize from first VM.
    const vm = data.vms?.[0];
    return {
      state: vm?.status?.toLowerCase() ?? 'not created',
      elapsed_ms: 0,
      history: [],
    };
  } catch {
    return { state: 'not created', elapsed_ms: 0, history: [] };
  }
}

// -- Real-time events (WebSocket /events) --

interface VmStateEvent {
  state: string;
  trigger?: string;
  message?: string;
}

let _eventWs: WebSocket | null = null;
const _vmStateCallbacks: ((payload: VmStateEvent) => void)[] = [];
const _downloadProgressCallbacks: ((progress: DownloadProgress) => void)[] = [];

function _connectEventWs() {
  if (_eventWs) return;
  if (!_token) return;
  const wsBase = _baseUrl.replace(/^http/, 'ws');
  const evUrl = `${wsBase}/events?token=${_token}`;
  console.log('[api] events-ws connecting url=%s', evUrl.replace(/token=[^&]+/, 'token=***'));
  _eventWs = new WebSocket(evUrl);
  _eventWs.onopen = () => {
    console.log('[api] events-ws connected');
  };
  _eventWs.onmessage = (ev) => {
    try {
      const msg = JSON.parse(ev.data as string);
      if (msg.type === 'vm-state-changed') {
        for (const cb of _vmStateCallbacks) cb(msg.payload);
      } else if (msg.type === 'download-progress') {
        for (const cb of _downloadProgressCallbacks) cb(msg.payload);
      }
    } catch {
      // Ignore malformed messages.
    }
  };
  _eventWs.onerror = () => {
    console.warn('[api] events-ws error');
  };
  _eventWs.onclose = () => {
    console.log('[api] events-ws closed, reconnecting in 5s');
    _eventWs = null;
    // Auto-reconnect after 5s if still connected.
    if (_connected) {
      setTimeout(() => _connectEventWs(), 5000);
    }
  };
}

/** Subscribe to VM state change events. Returns an unsubscribe function. */
export function onVmStateChanged(cb: (payload: VmStateEvent) => void): () => void {
  _vmStateCallbacks.push(cb);
  return () => {
    const i = _vmStateCallbacks.indexOf(cb);
    if (i >= 0) _vmStateCallbacks.splice(i, 1);
  };
}

/** Subscribe to download progress events. Returns an unsubscribe function. */
export function onDownloadProgress(cb: (progress: DownloadProgress) => void): () => void {
  _downloadProgressCallbacks.push(cb);
  return () => {
    const i = _downloadProgressCallbacks.indexOf(cb);
    if (i >= 0) _downloadProgressCallbacks.splice(i, 1);
  };
}

// -- Settings --

/** Load the merged settings tree (user + corp + defaults). */
export async function getSettings(): Promise<SettingsResponse> {
  const resp = await _get('/settings');
  return await resp.json();
}

/** Save settings changes. Returns the updated settings tree. */
export async function saveSettings(changes: Record<string, unknown>): Promise<SettingsResponse> {
  const resp = await _post('/settings', changes);
  return await resp.json();
}

/** List available security presets. */
export async function getPresets(): Promise<SecurityPreset[]> {
  const resp = await _get('/settings/presets');
  return await resp.json();
}

/** Apply a security preset by ID. Returns updated settings. */
export async function applyPreset(id: string): Promise<SettingsResponse> {
  const resp = await _post(`/settings/presets/${encodeURIComponent(id)}`);
  return await resp.json();
}

/** Validate config and return issues. */
export async function lintConfig(): Promise<ConfigIssue[]> {
  const resp = await _post('/settings/lint');
  return await resp.json();
}

// -- MCP config (mutations via settings API) --

/** Get MCP policy from settings. */
export async function getMcpPolicy(): Promise<McpPolicyInfo> {
  const resp = await _get('/settings');
  const settings: SettingsResponse = await resp.json();
  // Extract MCP policy from settings tree. The backend includes it in the response.
  return _extractMcpPolicy(settings);
}

function _extractMcpPolicy(settings: SettingsResponse): McpPolicyInfo {
  // Walk tree looking for mcp policy values; use defaults if not found.
  const policy: McpPolicyInfo = {
    global_policy: null,
    default_tool_permission: 'allow',
    blocked_servers: [],
    tool_permissions: {},
  };
  function walk(nodes: typeof settings.tree) {
    for (const node of nodes) {
      if (node.kind === 'leaf') {
        if (node.id === 'mcp.policy.global') {
          policy.global_policy = node.effective_value as string | null;
        } else if (node.id === 'mcp.policy.default_tool_permission') {
          policy.default_tool_permission = node.effective_value as string;
        }
      }
      if (node.kind === 'group' && 'children' in node) {
        walk(node.children);
      }
    }
  }
  walk(settings.tree);
  return policy;
}

/** Enable/disable an MCP server via settings. */
export async function setMcpServerEnabled(name: string, enabled: boolean): Promise<void> {
  await saveSettings({ [`mcp.servers.${name}.enabled`]: enabled });
}

/** Add an MCP server via settings. */
export async function addMcpServer(
  name: string,
  url: string,
  headers: Record<string, string>,
  bearerToken: string | null,
): Promise<void> {
  const changes: Record<string, unknown> = {
    [`mcp.servers.${name}.url`]: url,
    [`mcp.servers.${name}.enabled`]: true,
  };
  if (Object.keys(headers).length > 0) {
    changes[`mcp.servers.${name}.headers`] = headers;
  }
  if (bearerToken) {
    changes[`mcp.servers.${name}.bearer_token`] = bearerToken;
  }
  await saveSettings(changes);
}

/** Remove an MCP server via settings. */
export async function removeMcpServer(name: string): Promise<void> {
  await saveSettings({ [`mcp.servers.${name}`]: null });
}

/** Set the MCP global policy via settings. */
export async function setMcpGlobalPolicy(policy: string): Promise<void> {
  await saveSettings({ 'mcp.policy.global': policy });
}

/** Set the MCP default tool permission via settings. */
export async function setMcpDefaultPermission(permission: string): Promise<void> {
  await saveSettings({ 'mcp.policy.default_tool_permission': permission });
}

/** Set a per-tool MCP permission via settings. */
export async function setMcpToolPermission(tool: string, permission: string): Promise<void> {
  await saveSettings({ [`mcp.policy.tools.${tool}`]: permission });
}

// -- MCP runtime --

/** List configured MCP servers with tool counts (runtime). */
export async function getMcpServers(): Promise<McpServerInfo[]> {
  if (!_connected) return [];
  try {
    const resp = await _get('/mcp/servers');
    return await resp.json();
  } catch (err) {
    if (isNetworkError(err)) return [];
    throw err;
  }
}

/** List discovered MCP tools with cache/approval status (runtime). */
export async function getMcpTools(): Promise<McpToolInfo[]> {
  if (!_connected) return [];
  try {
    const resp = await _get('/mcp/tools');
    return await resp.json();
  } catch (err) {
    if (isNetworkError(err)) return [];
    throw err;
  }
}

/** Re-discover tools from MCP servers. */
export async function refreshMcpTools(server?: string): Promise<void> {
  await _post('/mcp/tools/refresh', server ? { server } : undefined);
}

/** Approve an MCP tool (writes tool cache). */
export async function approveMcpTool(name: string): Promise<void> {
  await _post(`/mcp/tools/${encodeURIComponent(name)}/approve`);
}

/** Call a built-in MCP file tool. */
export async function callMcpTool(name: string, args: Record<string, unknown>): Promise<unknown> {
  const resp = await _post(`/mcp/tools/${encodeURIComponent(name)}/call`, args);
  return await resp.json();
}

// -- Validation --

/** Validate an API key against a provider endpoint. */
export async function validateApiKey(provider: string, key: string): Promise<{ valid: boolean; message: string }> {
  try {
    const resp = await _post('/settings/validate-key', { provider, key });
    return await resp.json();
  } catch {
    return { valid: false, message: 'Validation failed (gateway unreachable)' };
  }
}

// -- Setup / Onboarding --

import type {
  SetupStateResponse,
  DetectedConfigSummary,
} from './types/onboarding';

/** Get setup/onboarding state (setup-state.json). */
export async function getSetupState(): Promise<SetupStateResponse> {
  const resp = await _get('/setup/state');
  return await resp.json();
}

/** Run host detection, write found values to settings, return summary. */
export async function runDetection(): Promise<DetectedConfigSummary> {
  const resp = await _get('/setup/detect');
  return await resp.json();
}

/** Mark GUI onboarding as completed. */
export async function completeOnboarding(): Promise<void> {
  await _post('/setup/complete');
}

// -- App actions --

/** Open a URL in the system default browser. */
export async function openUrl(url: string): Promise<void> {
  window.open(url, '_blank', 'noopener,noreferrer');
}

/** Check for app updates. Returns null if no update available. */
export async function checkForAppUpdate(): Promise<{ version: string; current_version: string } | null> {
  try {
    const resp = await _get('/update/check');
    return await resp.json();
  } catch {
    return null;
  }
}

// -- Files API (host-side VirtioFS) --

/** Sanitize a file path: allowlist [a-zA-Z0-9._\-/], strip leading slashes. */
export function sanitizePath(raw: string): string {
  return raw.replace(/[^a-zA-Z0-9._\-/]/g, '').replace(/\/+/g, '/').replace(/^\//, '');
}

/** List files in a VM workspace directory. */
export async function listFiles(id: string, path?: string, depth?: number): Promise<FileListResponse> {
  const params = new URLSearchParams();
  if (path) params.set('path', sanitizePath(path));
  if (depth != null) params.set('depth', String(depth));
  const qs = params.toString();
  const url = `/files/${encodeURIComponent(id)}${qs ? `?${qs}` : ''}`;
  const resp = await _get(url);
  return await resp.json();
}

/** Download a file from a VM workspace. Returns text, blob, and size. */
export async function getFileContent(id: string, path: string): Promise<FileContentResult> {
  const sanitized = sanitizePath(path);
  const resp = await fetch(`${_baseUrl}/files/${encodeURIComponent(id)}/content?path=${encodeURIComponent(sanitized)}`, {
    headers: { Authorization: `Bearer ${_token}` },
  });
  if (!resp.ok) {
    const body = await resp.text();
    throw new ApiError(resp.status, body);
  }
  const blob = await resp.blob();
  const text = await blob.text();
  return { text, blob, size: blob.size };
}

/** Upload a file to a VM workspace. */
export async function uploadFile(id: string, path: string, content: Blob | string): Promise<FileUploadResponse> {
  const sanitized = sanitizePath(path);
  const body = typeof content === 'string' ? new Blob([content]) : content;
  const resp = await fetch(`${_baseUrl}/files/${encodeURIComponent(id)}/content?path=${encodeURIComponent(sanitized)}`, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${_token}`,
      'Content-Type': 'application/octet-stream',
    },
    body,
  });
  if (!resp.ok) {
    const text = await resp.text();
    throw new ApiError(resp.status, text);
  }
  return await resp.json();
}

