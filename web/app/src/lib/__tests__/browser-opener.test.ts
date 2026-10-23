import { afterEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); invoke.mockClear(); });

describe('browser opener', () => {
  it('passes authenticated originating session context only for native guest requests', async () => {
    const token = 's'.repeat(64);
    invoke.mockResolvedValueOnce(token);
    vi.stubGlobal('window', { __TAURI_INTERNALS__: {}, location: new URL('tauri://localhost') });
    vi.stubGlobal('WebSocket', class { close() {} });
    vi.stubGlobal('fetch', vi.fn()
      .mockResolvedValueOnce(Response.json({ ok: true, version: '0.7.1' }))
      .mockResolvedValueOnce(Response.json({ service: 'running', gateway_version: '0.7.1', vm_count: 0, vms: [] })));
    const api = await import('../api');
    expect((await api.init()).connected).toBe(true);
    await api.openUrl('https://login.example/', 'vm-1');
    expect(invoke).toHaveBeenLastCalledWith('open_url', { url: 'https://login.example/',
      browser: { gatewayUrl: api.getBaseUrl(), token, vmId: 'vm-1' } });
    expect(invoke).toHaveBeenCalledWith('get_gateway_token');
    await api.openUrl('https://docs.example/');
    expect(invoke).toHaveBeenLastCalledWith('open_url', { url: 'https://docs.example/', browser: null });
  });

  it('keeps browser gesture opening synchronous and never exposes gateway credentials', async () => {
    const open = vi.fn();
    vi.stubGlobal('window', { open, location: new URL('http://127.0.0.1:55090') });
    const api = await import('../api');
    const result = api.openUrl('https://login.example/', 'vm-1');
    expect(open).toHaveBeenCalledWith('https://login.example/', '_blank', 'noopener,noreferrer');
    await result;
    expect(invoke).not.toHaveBeenCalled();
  });

  it('refuses a native session handoff without authentication', async () => {
    vi.stubGlobal('window', { __TAURI_INTERNALS__: {}, location: new URL('tauri://localhost') });
    const api = await import('../api');
    await expect(api.openUrl('https://login.example/', 'vm-1')).rejects.toThrow('Browser session unavailable');
    expect(invoke).not.toHaveBeenCalled();
  });
});
