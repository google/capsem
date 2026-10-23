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
    const child = {opener:{},document:{createElement:vi.fn(()=>({name:'',content:''})),head:{append:vi.fn()}},location:{replace:vi.fn()}};
    const open = vi.fn().mockReturnValue(child);
    vi.stubGlobal('window', { open, location: new URL('http://127.0.0.1:55090') });
    const api = await import('../api');
    const result = api.openUrl('https://login.example/', 'vm-1');
    expect(open).toHaveBeenCalledWith('', '_blank');
    expect(child.opener).toBeNull();
    expect(child.document.head.append).toHaveBeenCalledWith({name:'referrer',content:'no-referrer'});
    expect(child.location.replace).toHaveBeenCalledWith('https://login.example/');
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

it('does not report a blocked browser opening as success', async () => {
  vi.stubGlobal('window', {open:vi.fn().mockReturnValue(null),location:new URL('http://127.0.0.1:55090')});
  const api = await import('../api');
  await expect(api.openUrl('https://example.org/')).rejects.toThrow('blocked');
});

it('rejects unsafe URLs before invoking any browser capability', async () => {
  const open=vi.fn();vi.stubGlobal('window',{open,location:new URL('http://127.0.0.1:55090')});
  const api=await import('../api');
  await expect(api.openUrl('javascript:alert(1)')).rejects.toThrow('Unsupported');
  expect(open).not.toHaveBeenCalled();
});
