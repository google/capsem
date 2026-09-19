import { beforeEach, expect, it, vi } from 'vitest';
import { UpdateActionStatus } from '@capsem/sdk';
import { profile, updateStatusFixture } from './sdk-catalog-fixtures';

const mockFetch = vi.fn<typeof fetch>();
vi.stubGlobal('fetch', mockFetch);
vi.stubGlobal('WebSocket', class { close() {} });
const api = await import('../api');
function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}
beforeEach(async () => {
  mockFetch.mockReset();
  mockFetch.mockResolvedValueOnce(json({ ok: true, version: '1', service_socket: '/tmp/s' }))
    .mockResolvedValueOnce(json({ token: 'tok' }))
    .mockResolvedValueOnce(json({ service: 'running', gateway_version: '1', vm_count: 0, vms: [] }));
  await api.init();
  mockFetch.mockClear();
});

const reads = [
  { name: 'profiles', read: () => api.listProfiles(), path: '/profiles/list', value: { profiles: [profile] } },
  { name: 'update status', read: () => api.getUpdateStatus(), path: '/update/status', value: updateStatusFixture() },
];
it.each(reads)('reads typed $name through the SDK', async ({ read, path, value }) => {
  mockFetch.mockResolvedValueOnce(json(value));
  expect(await read()).toEqual(value);
  const [url, options] = mockFetch.mock.calls[0]!;
  expect(url).toBe(api.getBaseUrl() + path);
  expect(options?.redirect).toBe('manual');
  expect(new Headers(options?.headers).get('Authorization')).toBe('Bearer tok');
});
it.each(reads)('rejects incomplete $name', async ({ read }) => {
  mockFetch.mockResolvedValueOnce(json({}));
  await expect(read()).rejects.toThrow();
  expect(api.isConnected()).toBe(true);
});
it.each(reads)('refreshes a rotated token for $name once', async ({ read, value }) => {
  mockFetch.mockResolvedValueOnce(json({}, 401)).mockResolvedValueOnce(json({ token: 'rotated' }))
    .mockResolvedValueOnce(json(value));
  expect(await read()).toEqual(value);
  expect(mockFetch).toHaveBeenCalledTimes(3);
  expect(new Headers(mockFetch.mock.calls[2]?.[1]?.headers).get('Authorization')).toBe('Bearer rotated');
});
it('rejects unknown nested profile and update enums', async () => {
  mockFetch.mockResolvedValueOnce(json({ profiles: [{ ...profile, update_semantics: {
    ...profile.update_semantics, upgrade_action: 'unknown',
  } }] }));
  await expect(api.listProfiles()).rejects.toThrow();
  const updates = updateStatusFixture();
  mockFetch.mockResolvedValueOnce(json({ ...updates, binary: { ...updates.binary, state: 'unknown-state' } }));
  await expect(api.getUpdateStatus()).rejects.toThrow();
});
it('preserves update intent and validates its acknowledgement', async () => {
  const result = { status: UpdateActionStatus.PLANNED, command: { program: 'capsem', args: ['update', '--yes'] } };
  mockFetch.mockResolvedValueOnce(json(result));
  expect(await api.applyUpdate({ dry_run: true })).toEqual(result);
  const [url, options] = mockFetch.mock.calls[0]!;
  expect(url).toBe(api.getBaseUrl() + '/update/apply');
  expect(JSON.parse(String(options?.body))).toEqual({ dry_run: true });
  expect(options?.redirect).toBe('manual');
});
it('does not replay an update with a malformed success response', async () => {
  mockFetch.mockResolvedValueOnce(json({ status: 'succeeded' }));
  await expect(api.applyUpdate({ confirmed: true })).rejects.toThrow();
  expect(mockFetch).toHaveBeenCalledTimes(1);
});
