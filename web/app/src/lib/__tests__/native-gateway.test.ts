import { afterEach, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); invoke.mockReset(); });

it('keeps native HTTP and event WebSockets on the private selected origin', async () => {
  const gatewayUrl = 'http://127.0.0.1:65228';
  invoke.mockResolvedValueOnce({ gatewayUrl, token: 'a'.repeat(64) });
  vi.stubGlobal('window', { __TAURI_INTERNALS__: {}, location: new URL('tauri://localhost') });
  const sockets: string[] = [];
  vi.stubGlobal('WebSocket', class { constructor(url: string) { sockets.push(url); } close() {} });
  const fetch = vi.fn()
    .mockResolvedValueOnce(Response.json({ ok: true, version: '0.7.1' }))
    .mockResolvedValueOnce(Response.json({ service: 'running', gateway_version: '0.7.1', vm_count: 0, vms: [] }));
  vi.stubGlobal('fetch', fetch);
  const api = await import('../api');
  expect((await api.init()).connected).toBe(true);
  expect(api.getBaseUrl()).toBe(gatewayUrl);
  expect(fetch.mock.calls.every(([url]) => String(url).startsWith(gatewayUrl + '/'))).toBe(true);
  expect(sockets.length).toBe(1);
  expect(sockets[0].startsWith('ws://127.0.0.1:65228/')).toBe(true);
});

it('never probes the installed port when native connection metadata is invalid', async () => {
  invoke.mockResolvedValueOnce({ gatewayUrl: 'https://remote.example', token: 'a'.repeat(64) });
  vi.stubGlobal('window', { __TAURI_INTERNALS__: {}, location: new URL('tauri://localhost') });
  const fetch = vi.fn();
  vi.stubGlobal('fetch', fetch);
  const api = await import('../api');
  expect((await api.init()).connected).toBe(false);
  expect(fetch).not.toHaveBeenCalled();
});

it('does not replay an HTTP request using authority from a replacement origin', async () => {
  const gatewayUrl = 'http://127.0.0.1:65228';
  invoke.mockResolvedValueOnce({ gatewayUrl, token: 'a'.repeat(64) })
    .mockResolvedValueOnce({ gatewayUrl: 'http://127.0.0.1:65229', token: 'b'.repeat(64) });
  vi.stubGlobal('window', { __TAURI_INTERNALS__: {}, location: new URL('tauri://localhost') });
  vi.stubGlobal('WebSocket', class { close() {} });
  const fetch = vi.fn()
    .mockResolvedValueOnce(Response.json({ ok: true, version: '0.7.1' }))
    .mockResolvedValueOnce(Response.json({ service: 'running', gateway_version: '0.7.1', vm_count: 0, vms: [] }))
    .mockResolvedValueOnce(new Response('', { status: 401 }));
  vi.stubGlobal('fetch', fetch);
  const api = await import('../api');
  expect((await api.init()).connected).toBe(true);
  await expect(api.listPlugins()).rejects.toThrow('401');
  expect(fetch).toHaveBeenCalledTimes(3);
  expect(fetch.mock.calls.every(([url]) => String(url).startsWith(gatewayUrl + '/'))).toBe(true);
});
