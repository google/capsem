import { beforeEach, expect, it, vi } from 'vitest';
import { ServiceAvailability, VmAction, VmLifecycleState, type HypervisorInfo, type SandboxInfo, type VmStatsSummaryResponse } from '@capsem/sdk';

const mockFetch = vi.fn<typeof fetch>();
vi.stubGlobal('fetch', mockFetch);
vi.stubGlobal('WebSocket', class { close() {} });
const api = await import('../api');

const info: SandboxInfo = {
  id: 'vm 1', name: 'Demo', profile_id: 'code', pid: 123, status: VmLifecycleState.RUNNING,
  persistent: true, available_actions: [VmAction.PAUSE, VmAction.STOP, VmAction.FORK],
  total_input_tokens: 12, total_thinking_tokens: 3, total_output_tokens: 7, total_tool_calls: 2,
  ai: { models: [], mcp: [], model_call_count: 0, total_estimated_cost_usd: 0.001,
    total_input_tokens: 12, total_thinking_tokens: 3, total_output_tokens: 7, total_tool_calls: 2 },
  network: { allowed_requests: 2, denied_requests: 1, total_requests: 3, bytes_received: 100, bytes_sent: 50, errors: 0 },
  files: { actions: [], total_events: 0 },
};
const overview: HypervisorInfo = {
  service: ServiceAvailability.RUNNING, gateway_version: '1.0.0', vm_count: 1,
  vms: [{ id: info.id, profile_id: info.profile_id, status: info.status,
    persistent: true, available_actions: info.available_actions }],
  resource_summary: null, profiles: null, updates: null,
};
const stats: VmStatsSummaryResponse = {
  total_requests: 3, allowed_requests: 2, denied_requests: 1, total_input_tokens: 12,
  total_thinking_tokens: 3, total_output_tokens: 7, total_tool_calls: 2, total_estimated_cost: 0.001,
};
function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

beforeEach(async () => {
  mockFetch.mockReset();
  mockFetch.mockResolvedValueOnce(json({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
    .mockResolvedValueOnce(json({ token: 'tok' })).mockResolvedValueOnce(json(overview));
  await api.init();
  mockFetch.mockClear();
});

const reads = [
  { name: 'overview', read: () => api.getStatus(), value: overview, path: '/status' },
  { name: 'VM info', read: () => api.getVmInfo('vm 1'), value: info, path: '/vms/vm%201/info' },
  { name: 'stats summary', read: () => api.getVmStatsSummary('vm 1'), value: stats, path: '/vms/vm%201/stats/summary' },
];

it.each(reads)('reads typed $name through the authenticated SDK', async ({ read, value, path }) => {
  mockFetch.mockResolvedValueOnce(json(value));
  expect(await read()).toEqual(value);
  const [url, options] = mockFetch.mock.calls[0]!;
  expect(String(url)).toBe(api.getBaseUrl() + path);
  expect(new Headers(options?.headers).get('Authorization')).toBe('Bearer tok');
  expect(options?.redirect).toBe('manual');
});

it.each(reads)('rejects invalid $name without disconnecting or clearing state', async ({ read }) => {
  mockFetch.mockResolvedValueOnce(json({}));
  await expect(read()).rejects.toThrow();
  expect(api.isConnected()).toBe(true);
  expect(mockFetch).toHaveBeenCalledTimes(1);
});

it.each(reads)('surfaces malformed JSON from $name without disconnecting', async ({ read }) => {
  mockFetch.mockResolvedValueOnce(new Response('{invalid'));
  await expect(read()).rejects.toBeInstanceOf(SyntaxError);
  expect(api.isConnected()).toBe(true);
});

it.each(reads)('refreshes authentication once for $name', async ({ read, value }) => {
  mockFetch.mockResolvedValueOnce(json({ error: 'expired' }, 401))
    .mockResolvedValueOnce(json({ token: 'new-token' })).mockResolvedValueOnce(json(value));
  expect(await read()).toEqual(value);
  expect(mockFetch).toHaveBeenCalledTimes(3);
  expect(new Headers(mockFetch.mock.calls[2]?.[1]?.headers).get('Authorization')).toBe('Bearer new-token');
});

it.each(reads)('surfaces HTTP errors from $name', async ({ read }) => {
  mockFetch.mockResolvedValueOnce(json({ error: 'broken' }, 500));
  await expect(read()).rejects.toMatchObject({ name: 'ApiError', status: 500 });
  expect(api.isConnected()).toBe(true);
});

it.each(['overview', 'stats'] as const)('retains the offline fallback for a lost %s connection', async kind => {
  mockFetch.mockRejectedValueOnce(new TypeError('connection lost'));
  const result = kind === 'overview' ? await api.getStatus() : await api.getVmStatsSummary('vm 1');
  expect(result).toMatchObject(kind === 'overview' ? { service: 'offline', vms: [] } : { total_requests: 0 });
  expect(api.isConnected()).toBe(false);
  const offline = kind === 'overview' ? await api.getStatus() : await api.getVmStatsSummary('vm 1');
  expect(offline).toEqual(result);
  expect(mockFetch).toHaveBeenCalledTimes(1);
});

it('rejects unknown VM enums in the overview', async () => {
  mockFetch.mockResolvedValueOnce(json({ ...overview, vms: [{ ...overview.vms[0], status: 'future-state' }] }));
  await expect(api.getStatus()).rejects.toThrow();
  expect(api.isConnected()).toBe(true);
});

it('does not report a malformed background status probe as a lost connection', async () => {
  mockFetch.mockResolvedValueOnce(json({ ok: true })).mockResolvedValueOnce(json({}));
  await expect(api.healthCheck()).rejects.toThrow();
  expect(api.isConnected()).toBe(true);
});

it('reports connection loss during the background status probe', async () => {
  mockFetch.mockResolvedValueOnce(json({ ok: true })).mockRejectedValueOnce(new TypeError('network'));
  expect(await api.healthCheck()).toBe(false);
  expect(api.isConnected()).toBe(false);
});
