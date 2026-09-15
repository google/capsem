import { beforeEach, expect, it, vi } from 'vitest';
import { FileEntryType, type FileListResponse } from '@capsem/sdk';

const mockFetch = vi.fn<typeof fetch>();
vi.stubGlobal('fetch', mockFetch);
vi.stubGlobal('WebSocket', class { close() {} });
const api = await import('../api');
function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}
const listing: FileListResponse = { entries: [{ name: 'data.bin', path: 'dir/data.bin',
  type: FileEntryType.FILE, size: 3, mtime: 123, mime: null, is_text: false, children: null }] };

beforeEach(async () => {
  mockFetch.mockReset();
  mockFetch.mockResolvedValueOnce(json({ ok: true, version: '1', service_socket: '/tmp/s' }))
    .mockResolvedValueOnce(json({ token: 'tok' }))
    .mockResolvedValueOnce(json({ service: 'running', gateway_version: '1', vm_count: 0, vms: [] }));
  await api.init();
  mockFetch.mockClear();
});

it('reads typed file entries, nullable metadata and bounded-depth intent', async () => {
  mockFetch.mockResolvedValueOnce(json(listing));
  expect(await api.listFiles('vm 1', '/dir//', 4)).toEqual(listing);
  const [url, options] = mockFetch.mock.calls[0]!;
  const parsed = new URL(String(url));
  expect(parsed.pathname).toBe('/vms/vm%201/files/list');
  expect(parsed.searchParams.get('path')).toBe('dir/');
  expect(parsed.searchParams.get('depth')).toBe('4');
  expect(options?.redirect).toBe('manual');
});

it('omits unset directory and depth options', async () => {
  mockFetch.mockResolvedValueOnce(json({ entries: [] }));
  expect(await api.listFiles('vm 1')).toEqual({ entries: [] });
  expect(new URL(String(mockFetch.mock.calls[0]?.[0])).search).toBe('');
});

it('rejects malformed file metadata and unknown entry enums', async () => {
  for (const change of [{ size: '3' }, { type: 'device' }]) {
    mockFetch.mockResolvedValueOnce(json({ entries: [{ ...listing.entries[0], ...change }] }));
    await expect(api.listFiles('vm 1')).rejects.toThrow();
  }
  expect(api.isConnected()).toBe(true);
});

it('preserves arbitrary download bytes and derives the displayed size from them', async () => {
  const bytes = new Uint8Array([0, 255, 65]);
  mockFetch.mockResolvedValueOnce(new Response(bytes));
  const result = await api.getFileContent('vm 1', '/dir/data.bin');
  expect(new Uint8Array(await result.blob.arrayBuffer())).toEqual(bytes);
  expect(result.size).toBe(3);
  expect(result.text).toBe(new TextDecoder().decode(bytes));
  const [url, options] = mockFetch.mock.calls[0]!;
  expect(new URL(String(url)).searchParams.get('path')).toBe('dir/data.bin');
  expect(new Headers(options?.headers).get('Accept')).toBe('application/octet-stream');
  expect(options?.redirect).toBe('manual');
});

it('carries validated file-list MIME metadata into preview blobs', async () => {
  const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"/>';
  mockFetch.mockResolvedValueOnce(new Response(svg));
  const result = await api.getFileContent('vm 1', 'preview.svg', 'image/svg+xml');
  expect(result.blob.type).toBe('image/svg+xml');
  expect(await result.blob.text()).toBe(svg);
});

it.each(['binary', 'text'] as const)('uploads %s without changing its bytes', async kind => {
  const bytes = kind === 'binary' ? new Uint8Array([0, 255, 65]) : new TextEncoder().encode('café');
  const body = kind === 'binary' ? new Blob([bytes]) : 'café';
  mockFetch.mockResolvedValueOnce(json({ success: true, size: bytes.length }));
  expect(await api.uploadFile('vm 1', '/dir/data.bin', body)).toEqual({ success: true, size: bytes.length });
  const [url, options] = mockFetch.mock.calls[0]!;
  expect(new URL(String(url)).searchParams.get('path')).toBe('dir/data.bin');
  expect(options?.body).toEqual(bytes);
  expect(new Headers(options?.headers).get('Content-Type')).toBe('application/octet-stream');
  expect(options?.redirect).toBe('manual');
});

it('retries binary upload once after token refresh with identical bytes', async () => {
  const bytes = new Uint8Array([0, 255, 65]);
  mockFetch.mockResolvedValueOnce(json({}, 401)).mockResolvedValueOnce(json({ token: 'rotated' }))
    .mockResolvedValueOnce(json({ success: true, size: 3 }));
  expect(await api.uploadFile('vm 1', 'data.bin', new Blob([bytes]))).toEqual({ success: true, size: 3 });
  expect(mockFetch).toHaveBeenCalledTimes(3);
  expect(mockFetch.mock.calls[0]?.[1]?.body).toEqual(bytes);
  expect(mockFetch.mock.calls[2]?.[1]?.body).toEqual(bytes);
  expect(new Headers(mockFetch.mock.calls[2]?.[1]?.headers).get('Authorization')).toBe('Bearer rotated');
});

it('rejects malformed upload acknowledgements without replay', async () => {
  mockFetch.mockResolvedValueOnce(json({ success: true }));
  await expect(api.uploadFile('vm 1', 'data.bin', 'data')).rejects.toThrow();
  expect(mockFetch).toHaveBeenCalledTimes(1);
});

it('surfaces the stopped-VM security-ledger conflict unchanged', async () => {
  mockFetch.mockResolvedValueOnce(new Response('file import/export requires a running sandbox security ledger', { status: 409 }));
  await expect(api.getFileContent('vm 1', 'data.bin')).rejects.toMatchObject({ name: 'ApiError', status: 409 });
  expect(mockFetch).toHaveBeenCalledTimes(1);
});
