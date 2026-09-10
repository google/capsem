import { beforeEach, expect, it, vi } from 'vitest';
import { HostLogSource, SnapshotOrigin, type VmStatsDetailResponse, type SnapshotsStatus } from '@capsem/sdk';

const mockFetch = vi.fn<typeof fetch>();
vi.stubGlobal('fetch', mockFetch);
vi.stubGlobal('WebSocket', class { close() {} });
const api = await import('../api');
function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

const stats: VmStatsDetailResponse = {
  model_stats: [{ provider: 'google', model: 'fixture-model', call_count: 1, duration_ms: 25,
    input_tokens: 12, output_tokens: 7, estimated_cost_usd: 0.001 }],
  model_events: [], tool_events: [], http_events: [], dns_events: [], file_events: [],
  process_events: [], audit_events: [], credential_events: [], body_blobs: {},
};
const snapshots: SnapshotsStatus = {
  total: 1, auto_count: 1, manual_count: 0, manual_available: 12,
  snapshots: [{ checkpoint: 'cp-0', slot: 0, origin: SnapshotOrigin.AUTO, timestamp: 'unix:1' }],
};

beforeEach(async () => {
  mockFetch.mockReset();
  mockFetch.mockResolvedValueOnce(json({ ok: true, version: '1', service_socket: '/tmp/s' }))
    .mockResolvedValueOnce(json({ token: 'tok' }))
    .mockResolvedValueOnce(json({ service: 'running', gateway_version: '1', vm_count: 0, vms: [] }));
  await api.init();
  mockFetch.mockClear();
});

const reads = [
  { name: 'VM logs', read: () => api.getVmLogs('vm 1'), path: '/vms/vm%201/logs', value: { logs: 'session log', serial_logs: null, process_logs: 'process log' } },
  { name: 'stats detail', read: () => api.getVmStatsDetail('vm 1'), path: '/vms/vm%201/stats/detail', value: stats },
  { name: 'snapshot status', read: () => api.getVmSnapshotStatus('vm 1'), path: '/vms/vm%201/snapshots/status', value: snapshots },
  { name: 'snapshot list', read: () => api.listVmSnapshots('vm 1'), path: '/vms/vm%201/snapshots/list', value: { total: 1, snapshots: snapshots.snapshots } },
];

it.each(reads)('validates and authenticates $name', async ({ read, value, path }) => {
  mockFetch.mockResolvedValueOnce(json(value));
  expect(await read()).toEqual(value);
  const [url, options] = mockFetch.mock.calls[0]!;
  expect(String(url)).toBe(api.getBaseUrl() + path);
  expect(new Headers(options?.headers).get('Authorization')).toBe('Bearer tok');
  expect(options?.redirect).toBe('manual');
});

it.each(reads)('surfaces incomplete $name without reporting a lost connection', async ({ read }) => {
  mockFetch.mockResolvedValueOnce(json({}));
  await expect(read()).rejects.toThrow();
  expect(api.isConnected()).toBe(true);
});

it.each(reads)('refreshes a rotated token once for $name', async ({ read, value }) => {
  mockFetch.mockResolvedValueOnce(json({}, 401)).mockResolvedValueOnce(json({ token: 'rotated' }))
    .mockResolvedValueOnce(json(value));
  expect(await read()).toEqual(value);
  expect(mockFetch).toHaveBeenCalledTimes(3);
  expect(new Headers(mockFetch.mock.calls[2]?.[1]?.headers).get('Authorization')).toBe('Bearer rotated');
});

it('reads service logs through the typed hypervisor log endpoint', async () => {
  mockFetch.mockResolvedValueOnce(json({ source: HostLogSource.SERVICE, text: 'service log' }));
  expect(await api.getServiceLogs()).toBe('service log');
  expect(mockFetch.mock.calls[0]?.[0]).toBe(api.getBaseUrl() + '/host-logs/service');
});

it('rejects invalid host log sources and nested stats fields', async () => {
  mockFetch.mockResolvedValueOnce(json({ source: 'unknown', text: 'log' }));
  await expect(api.getServiceLogs()).rejects.toThrow();
  mockFetch.mockResolvedValueOnce(json({ ...stats, model_stats: [{ provider: 'google', call_count: '1' }] }));
  await expect(api.getVmStatsDetail('vm 1')).rejects.toThrow();
  expect(api.isConnected()).toBe(true);
});

it('rejects an unknown snapshot origin', async () => {
  mockFetch.mockResolvedValueOnce(json({ ...snapshots, snapshots: [{ ...snapshots.snapshots[0], origin: 'future' }] }));
  await expect(api.getVmSnapshotStatus('vm 1')).rejects.toThrow();
});

it.each(['logs', 'stats', 'service'] as const)('retains the offline fallback for %s connection failures', async kind => {
  const read = kind === 'logs' ? () => api.getVmLogs('vm 1')
    : kind === 'stats' ? () => api.getVmStatsDetail('vm 1') : () => api.getServiceLogs();
  mockFetch.mockRejectedValueOnce(new TypeError('connection lost'));
  const result = await read();
  expect(api.isConnected()).toBe(false);
  expect(await read()).toEqual(result);
  expect(mockFetch).toHaveBeenCalledTimes(1);
});

it('executes with a typed response and preserves the requested command timeout', async () => {
  const result = { stdout: 'hello', stderr: '', exit_code: 0, truncated: false };
  mockFetch.mockResolvedValueOnce(json(result));
  expect(await api.execCommand('vm 1', 'echo hello', 10)).toEqual(result);
  const [url, options] = mockFetch.mock.calls[0]!;
  expect(String(url)).toBe(api.getBaseUrl() + '/vms/vm%201/exec');
  expect(JSON.parse(String(options?.body))).toEqual({ command: 'echo hello', timeout_secs: 10 });
  expect(options?.redirect).toBe('manual');
});

it('rejects malformed exec output without replaying the command', async () => {
  mockFetch.mockResolvedValueOnce(json({ stdout: 'hello' }));
  await expect(api.execCommand('vm 1', 'echo hello')).rejects.toThrow();
  expect(mockFetch).toHaveBeenCalledTimes(1);
});
