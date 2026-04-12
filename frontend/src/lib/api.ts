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
} from './types/gateway';
import {
  mockVMs,
  mockModelStats,
  mockToolCalls,
  mockNetworkEvents,
  mockFileEvents,
  mockLogEntries,
  mockFileTree,
  executeMockQuery,
  type MockVM,
  type MockLogEntry,
  type MockFileNode,
} from './mock';

// -- Module state (never exported directly) --

let _token: string | null = null;
let _baseUrl = 'http://127.0.0.1:19222';
let _connected = false;

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
  try {
    // Probe health first (unauthenticated)
    const healthResp = await fetch(`${_baseUrl}/`);
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
    return { connected: true, reachable: true, version: health.version };
  } catch {
    _connected = false;
    _token = null;
    return { connected: false, reachable: false, version: null };
  }
}

export async function healthCheck(): Promise<boolean> {
  try {
    const resp = await fetch(`${_baseUrl}/`);
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
  if (!_connected) return mockStatus();
  try {
    const resp = await _get('/status');
    return await resp.json();
  } catch (err) {
    if (isNetworkError(err)) {
      _connected = false;
      return mockStatus();
    }
    throw err;
  }
}

function mockStatus(): StatusResponse {
  return {
    service: 'mock',
    gateway_version: '0.0.0-mock',
    vm_count: mockVMs.length,
    vms: mockVMs.map(vm => ({
      id: vm.id,
      name: vm.name,
      status: vm.status.charAt(0).toUpperCase() + vm.status.slice(1),
      persistent: vm.persistent,
    })),
    resource_summary: {
      total_ram_mb: mockVMs.reduce((a, v) => a + v.ram, 0),
      total_cpus: mockVMs.reduce((a, v) => a + v.cpus, 0),
      running_count: mockVMs.filter(v => v.status === 'running').length,
      stopped_count: mockVMs.filter(v => v.status === 'stopped').length,
      suspended_count: 0,
    },
  };
}

// -- VM lifecycle --

export async function provisionVm(opts: ProvisionRequest): Promise<ProvisionResponse> {
  const resp = await _post('/provision', opts);
  return await resp.json();
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

export async function persistVm(id: string): Promise<void> {
  await _post(`/persist/${encodeURIComponent(id)}`);
}

export async function forkVm(id: string, opts: ForkRequest): Promise<ForkResponse> {
  const resp = await _post(`/fork/${encodeURIComponent(id)}`, opts);
  return await resp.json();
}

// -- VM inspection --

export async function getVmLogs(id: string): Promise<MockLogEntry[]> {
  if (!_connected) return mockLogEntries;
  try {
    const resp = await _get(`/logs/${encodeURIComponent(id)}`);
    return await resp.json();
  } catch (err) {
    if (isNetworkError(err)) {
      _connected = false;
      return mockLogEntries;
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
  if (!_connected) return executeMockQuery(sql);
  try {
    const resp = await _post(`/inspect/${encodeURIComponent(id)}`, { sql });
    return await resp.json();
  } catch (err) {
    if (isNetworkError(err)) {
      _connected = false;
      return executeMockQuery(sql);
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

// -- Terminal --

export function getTerminalWsUrl(id: string): string {
  const wsBase = _baseUrl.replace(/^http/, 'ws');
  return `${wsBase}/terminal/${encodeURIComponent(id)}?token=${_token}`;
}

// -- Re-exports for mock fallback --

export { mockVMs, mockModelStats, mockToolCalls, mockNetworkEvents, mockFileEvents, mockLogEntries, mockFileTree };
export type { MockVM, MockLogEntry, MockFileNode };
