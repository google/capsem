// Gateway API client. Token is module-scoped -- never in localStorage, DOM, logs, or URLs.

import { recordWsEvent } from './tauri-log';
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
} from './types/settings';
import type {
  DownloadProgress,
  McpServerInfo,
  McpToolInfo,
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

export type PluginMode = 'allow' | 'ask' | 'block' | 'disable' | 'rewrite';
export type PluginDetectionLevel = 'informational' | 'low' | 'medium' | 'high' | 'critical';
export type PluginStage = 'preprocess' | 'postprocess' | 'pre_and_post';

export interface PluginConfig {
  mode: PluginMode;
  detection_level: PluginDetectionLevel;
}

export interface PluginScope {
  kind: 'profile';
  profile_id: string;
}

export interface BrokeredCredentialStatus {
  provider: string | null;
  credential_ref: string;
  observed_count: number;
  substituted_count: number;
  last_seen: string | null;
}

export interface PluginRuntimeStatus {
  enabled: boolean;
  event_count: number;
  detection_count: number;
  block_count: number;
  rewrite_count: number;
  last_error: string | null;
  brokered_credentials: BrokeredCredentialStatus[];
}

export interface PluginInfo {
  id: string;
  config: PluginConfig;
  default_config: PluginConfig;
  overridden: boolean;
  scope: PluginScope;
  description: string;
  stage: PluginStage;
  version: string;
  runtime: PluginRuntimeStatus;
}

export interface PluginListResponse {
  scope: PluginScope;
  plugins: PluginInfo[];
}

export interface McpServerEditRequest {
  url?: string;
  headers?: Record<string, string>;
  enabled?: boolean;
}

export interface ProfileSummary {
  id: string;
  name: string;
  description: string;
  source: string;
  rule_count: number;
  default_rule_count: number;
  plugin_count: number;
  mcp_server_count: number;
}

export interface ProfilesListResponse {
  profiles: ProfileSummary[];
}

export interface ProfileInfoResponse {
  profile: ProfileSummary;
}

export interface ProfileValidateRequest {
  toml?: string;
  profile?: Record<string, unknown>;
}

export interface ProfileValidateResponse {
  valid: boolean;
  profile_id: string;
}

export type SecurityRuleAction = 'allow' | 'ask' | 'block' | 'preprocess' | 'rewrite' | 'postprocess';
export type SecurityRuleDetectionLevel = 'informational' | 'low' | 'medium' | 'high' | 'critical';

export interface EnforcementRuleInfo {
  rule_id: string;
  source: string;
  provider: string;
  namespace: string;
  rule_key: string;
  default_rule: boolean;
  name: string;
  action: SecurityRuleAction;
  match: string;
  detection_level?: SecurityRuleDetectionLevel;
  priority: number;
  corp_locked: boolean;
  reason?: string;
}

export interface EnforcementRuleListResponse {
  profile_id: string;
  rules: EnforcementRuleInfo[];
}

export interface EnforcementInfoResponse {
  profile_id: string;
  rule_count: number;
  default_rule_count: number;
  custom_rule_count: number;
  detection_rule_count: number;
  corp_locked_rule_count: number;
  source_counts: Record<string, number>;
  action_counts: Record<string, number>;
}

export type DetectionRuleInfo = EnforcementRuleInfo;
export type DetectionRuleListResponse = EnforcementRuleListResponse;
export type DetectionInfoResponse = EnforcementInfoResponse;

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

async function _patch(path: string, body?: unknown): Promise<Response> {
  const resp = await fetch(`${_baseUrl}${path}`, {
    method: 'PATCH',
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

async function _put(path: string, body?: unknown): Promise<Response> {
  const resp = await fetch(`${_baseUrl}${path}`, {
    method: 'PUT',
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

async function routeJson(path: string): Promise<unknown> {
  const resp = await _get(path);
  return await resp.json();
}

function settledValue(result: PromiseSettledResult<unknown>): unknown {
  if (result.status === 'fulfilled') return result.value;
  return { error: result.reason instanceof Error ? result.reason.message : String(result.reason) };
}

export async function debugSnapshot(): Promise<unknown> {
  const [status, profilesStatus, corpInfo] = await Promise.allSettled([
    getStatus(),
    routeJson('/profiles/status'),
    routeJson('/corp/info'),
  ]);
  return {
    generated_at: new Date().toISOString(),
    connected: _connected,
    base_url: _baseUrl,
    status: settledValue(status),
    profiles_status: settledValue(profilesStatus),
    corp_info: settledValue(corpInfo),
  };
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
  const resp = await _post('/vms/create', opts);
  const result = await resp.json();
  console.log('[api] provisionVm result:', result);
  return result;
}

export async function runVm(opts: ProvisionRequest): Promise<ProvisionResponse> {
  const resp = await _post('/run', opts);
  return await resp.json();
}

export async function stopVm(id: string): Promise<void> {
  await _post(`/vms/${encodeURIComponent(id)}/stop`);
}

export async function suspendVm(id: string): Promise<void> {
  await _post(`/vms/${encodeURIComponent(id)}/pause`);
}

export async function deleteVm(id: string): Promise<void> {
  await _delete(`/vms/${encodeURIComponent(id)}/delete`);
}

export async function resumeVm(name: string): Promise<void> {
  await _post(`/vms/${encodeURIComponent(name)}/resume`);
}

export async function persistVm(id: string, name: string): Promise<void> {
  await _post(`/vms/${encodeURIComponent(id)}/save`, { name });
}

export async function forkVm(id: string, opts: ForkRequest): Promise<ForkResponse> {
  const resp = await _post(`/vms/${encodeURIComponent(id)}/fork`, opts);
  return await resp.json();
}

// -- VM inspection --

/** Raw log response from GET /vms/{id}/logs. */
export interface RawLogsResponse {
  logs: string;
  serial_logs: string | null;
  process_logs: string | null;
}

export async function getVmLogs(id: string): Promise<RawLogsResponse> {
  if (!_connected) return { logs: '', serial_logs: null, process_logs: null };
  try {
    const resp = await _get(`/vms/${encodeURIComponent(id)}/logs`);
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
  const resp = await _post(`/vms/${encodeURIComponent(id)}/exec`, {
    command,
    timeout_secs: timeoutSecs,
  });
  return await resp.json();
}

export async function inspectQuery(id: string, sql: string): Promise<InspectResponse> {
  if (!_connected) return { columns: [], rows: [] };
  try {
    const resp = await _post(`/vms/${encodeURIComponent(id)}/inspect`, { sql });
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
  const resp = await _post(`/vms/${encodeURIComponent(id)}/files/read`, { path });
  return await resp.json();
}

export async function writeFile(id: string, path: string, content: string): Promise<void> {
  await _post(`/vms/${encodeURIComponent(id)}/files/write`, { path, content });
}

// -- Images --

export async function getImages(): Promise<{ images: { name: string }[] }> {
  const resp = await _get('/images');
  return await resp.json();
}

// -- Config --

export async function reloadProfile(profileId = 'code'): Promise<void> {
  await _post(`/profiles/${encodeURIComponent(profileId)}/reload`);
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
    // T5/F3: feed __capsemDebug.lastWsEvents ring buffer (no-op in
    // production unless ?debug=1 set installs the global).
    recordWsEvent({ kind: 'message', bytes: data.length, ts: Date.now() });
    if (_termWaiter) {
      const w = _termWaiter;
      _termWaiter = null;
      w(data);
    } else {
      _termBuffer.push(...data);
    }
  };
  _termWs.onclose = () => {
    recordWsEvent({ kind: 'close', ts: Date.now() });
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
    const path = id ? `/vms/${encodeURIComponent(id)}/status` : '/status';
    const resp = await _get(path);
    const data = await resp.json();
    // /vms/{id}/status returns runtime state; extract optional transition history.
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
  const resp = await _get('/settings/info');
  return await resp.json();
}

/** Save settings changes. Returns the updated settings tree. */
export async function saveSettings(changes: Record<string, unknown>): Promise<SettingsResponse> {
  const resp = await _patch('/settings/edit', changes);
  return await resp.json();
}

// -- Profiles --

export async function listProfiles(): Promise<ProfilesListResponse> {
  const resp = await _get('/profiles/list');
  return await resp.json();
}

export async function getProfileInfo(profileId: string): Promise<ProfileInfoResponse> {
  const resp = await _get(`/profiles/${encodeURIComponent(profileId)}/info`);
  return await resp.json();
}

export async function validateProfile(
  profileId: string,
  request: ProfileValidateRequest = {},
): Promise<ProfileValidateResponse> {
  const resp = await _post(`/profiles/${encodeURIComponent(profileId)}/validate`, request);
  return await resp.json();
}

export async function createProfile(request: Record<string, unknown>): Promise<unknown> {
  const resp = await _post('/profiles/create', request);
  return await resp.json();
}

export async function editProfile(profileId: string, request: Record<string, unknown>): Promise<unknown> {
  const resp = await _patch(`/profiles/${encodeURIComponent(profileId)}/edit`, request);
  return await resp.json();
}

export async function deleteProfile(profileId: string): Promise<unknown> {
  const resp = await _delete(`/profiles/${encodeURIComponent(profileId)}/delete`);
  return await resp.json();
}

export async function cloneProfile(profileId: string, request: Record<string, unknown>): Promise<unknown> {
  const resp = await _post(`/profiles/${encodeURIComponent(profileId)}/clone`, request);
  return await resp.json();
}

export async function getProfileSkillsInfo(profileId: string): Promise<unknown> {
  const resp = await _get(`/profiles/${encodeURIComponent(profileId)}/skills/info`);
  return await resp.json();
}

export async function listProfileSkills(profileId: string): Promise<unknown> {
  const resp = await _get(`/profiles/${encodeURIComponent(profileId)}/skills/list`);
  return await resp.json();
}

export async function addProfileSkill(profileId: string, request: Record<string, unknown>): Promise<unknown> {
  const resp = await _post(`/profiles/${encodeURIComponent(profileId)}/skills/add`, request);
  return await resp.json();
}

export async function editProfileSkill(
  profileId: string,
  skillId: string,
  request: Record<string, unknown>,
): Promise<unknown> {
  const resp = await _patch(
    `/profiles/${encodeURIComponent(profileId)}/skills/${encodeURIComponent(skillId)}/edit`,
    request,
  );
  return await resp.json();
}

export async function deleteProfileSkill(profileId: string, skillId: string): Promise<unknown> {
  const resp = await _delete(
    `/profiles/${encodeURIComponent(profileId)}/skills/${encodeURIComponent(skillId)}/delete`,
  );
  return await resp.json();
}

export async function getProfileAssetsInfo(profileId: string): Promise<unknown> {
  const resp = await _get(`/profiles/${encodeURIComponent(profileId)}/assets/info`);
  return await resp.json();
}

export async function editProfileAssets(profileId: string, request: Record<string, unknown>): Promise<unknown> {
  const resp = await _patch(`/profiles/${encodeURIComponent(profileId)}/assets/edit`, request);
  return await resp.json();
}

export async function getProfilePluginsInfo(profileId: string): Promise<unknown> {
  const resp = await _get(`/profiles/${encodeURIComponent(profileId)}/plugins/info`);
  return await resp.json();
}

export async function getProfileMcpInfo(profileId: string): Promise<unknown> {
  const resp = await _get(`/profiles/${encodeURIComponent(profileId)}/mcp/info`);
  return await resp.json();
}

// -- Enforcement rules --

export async function listEnforcementRules(profileId: string): Promise<EnforcementRuleListResponse> {
  const resp = await _get(`/profiles/${encodeURIComponent(profileId)}/enforcement/rules/list`);
  return await resp.json();
}

export async function getEnforcementInfo(profileId: string): Promise<EnforcementInfoResponse> {
  const resp = await _get(`/profiles/${encodeURIComponent(profileId)}/enforcement/info`);
  return await resp.json();
}

// -- Detection rules --

export async function listDetectionRules(profileId: string): Promise<DetectionRuleListResponse> {
  const resp = await _get(`/profiles/${encodeURIComponent(profileId)}/detection/rules/list`);
  return await resp.json();
}

export async function getDetectionInfo(profileId: string): Promise<DetectionInfoResponse> {
  const resp = await _get(`/profiles/${encodeURIComponent(profileId)}/detection/info`);
  return await resp.json();
}

// -- Runtime ledger --

export async function getSecurityLatest(): Promise<unknown> {
  const resp = await _get('/security/latest');
  return await resp.json();
}

export async function getSecurityStatus(): Promise<unknown> {
  const resp = await _get('/security/status');
  return await resp.json();
}

export async function getEnforcementLatest(): Promise<unknown> {
  const resp = await _get('/enforcement/latest');
  return await resp.json();
}

export async function getEnforcementStatus(): Promise<unknown> {
  const resp = await _get('/enforcement/status');
  return await resp.json();
}

export async function getDetectionLatest(): Promise<unknown> {
  const resp = await _get('/detection/latest');
  return await resp.json();
}

export async function getDetectionStatus(): Promise<unknown> {
  const resp = await _get('/detection/status');
  return await resp.json();
}

// -- Plugins --

export async function listPlugins(profileId: string): Promise<PluginListResponse> {
  const resp = await _get(`/profiles/${encodeURIComponent(profileId)}/plugins/list`);
  return await resp.json();
}

export async function updatePlugin(
  profileId: string,
  pluginId: string,
  update: Partial<PluginConfig>,
): Promise<PluginInfo> {
  const resp = await _patch(
    `/profiles/${encodeURIComponent(profileId)}/plugins/${encodeURIComponent(pluginId)}/edit`,
    update,
  );
  return await resp.json();
}

// -- MCP config --

/** Add or replace an MCP server in a profile. */
export async function upsertMcpServer(
  profileId: string,
  serverId: string,
  url: string,
  headers: Record<string, string>,
): Promise<McpServerInfo> {
  const resp = await _put(
    `/profiles/${encodeURIComponent(profileId)}/mcp/servers/${encodeURIComponent(serverId)}/edit`,
    { url, headers, enabled: true } satisfies McpServerEditRequest,
  );
  return await resp.json();
}

/** Enable/disable or otherwise update an MCP server in a profile. */
export async function updateMcpServer(
  profileId: string,
  serverId: string,
  update: McpServerEditRequest,
): Promise<McpServerInfo> {
  const resp = await _put(
    `/profiles/${encodeURIComponent(profileId)}/mcp/servers/${encodeURIComponent(serverId)}/edit`,
    update,
  );
  return await resp.json();
}

/** Remove an MCP server from a profile. */
export async function deleteMcpServer(profileId: string, serverId: string): Promise<void> {
  await _delete(
    `/profiles/${encodeURIComponent(profileId)}/mcp/servers/${encodeURIComponent(serverId)}/delete`,
  );
}

// -- MCP runtime --

/** List configured MCP servers with tool counts (runtime). */
export async function getMcpServers(profileId: string): Promise<McpServerInfo[]> {
  if (!_connected) return [];
  try {
    const resp = await _get(`/profiles/${encodeURIComponent(profileId)}/mcp/servers/list`);
    return await resp.json();
  } catch (err) {
    if (isNetworkError(err)) return [];
    throw err;
  }
}

/** List discovered MCP tools with cache/approval status (runtime). */
export async function getMcpTools(profileId: string, serverId: string): Promise<McpToolInfo[]> {
  if (!_connected) return [];
  try {
    const resp = await _get(
      `/profiles/${encodeURIComponent(profileId)}/mcp/servers/${encodeURIComponent(serverId)}/tools/list`,
    );
    return await resp.json();
  } catch (err) {
    if (isNetworkError(err)) return [];
    throw err;
  }
}

/** Re-discover tools from MCP servers. */
export async function refreshMcpTools(profileId: string, serverId: string): Promise<void> {
  await _post(
    `/profiles/${encodeURIComponent(profileId)}/mcp/servers/${encodeURIComponent(serverId)}/refresh`,
  );
}

/** Edit MCP tool mechanics such as cache approval. */
export async function approveMcpTool(
  profileId: string,
  serverId: string,
  toolId: string,
): Promise<void> {
  await _patch(
    `/profiles/${encodeURIComponent(profileId)}/mcp/servers/${encodeURIComponent(serverId)}/tools/${encodeURIComponent(toolId)}/edit`,
    { approved: true },
  );
}

/** Call a built-in MCP file tool. */
export async function callMcpTool(
  profileId: string,
  serverId: string,
  toolId: string,
  args: Record<string, unknown>,
): Promise<unknown> {
  const resp = await _post(
    `/profiles/${encodeURIComponent(profileId)}/mcp/servers/${encodeURIComponent(serverId)}/tools/${encodeURIComponent(toolId)}/call`,
    args,
  );
  return await resp.json();
}

// -- Assets --

import type { AssetStatusResponse } from './types/assets';

/** Get first-class VM asset status. */
export async function getAssetsStatus(profileId = 'code'): Promise<AssetStatusResponse> {
  const resp = await _get(`/profiles/${encodeURIComponent(profileId)}/assets/status`);
  return await resp.json();
}

/** Ensure missing/corrupt VM assets, then return refreshed status. */
export async function ensureAssets(profileId = 'code'): Promise<AssetStatusResponse> {
  const resp = await _post(`/profiles/${encodeURIComponent(profileId)}/assets/ensure`, {});
  return await resp.json();
}

// -- App actions --

/** Open a URL in the system default browser. Routes through the Tauri IPC
 * inside the desktop shell (where `window.open` is a no-op) and falls back to
 * a new tab in the browser. */
export async function openUrl(url: string): Promise<void> {
  if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('open_url', { url });
    return;
  }
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
  const url = `/vms/${encodeURIComponent(id)}/files/list${qs ? `?${qs}` : ''}`;
  const resp = await _get(url);
  return await resp.json();
}

/** Download a file from a VM workspace. Returns text, blob, and size. */
export async function getFileContent(id: string, path: string): Promise<FileContentResult> {
  const sanitized = sanitizePath(path);
  const resp = await fetch(`${_baseUrl}/vms/${encodeURIComponent(id)}/files/content?path=${encodeURIComponent(sanitized)}`, {
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
  const resp = await fetch(`${_baseUrl}/vms/${encodeURIComponent(id)}/files/content?path=${encodeURIComponent(sanitized)}`, {
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
