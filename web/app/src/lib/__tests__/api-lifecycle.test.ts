import { describe, it, expect, vi, beforeEach } from 'vitest';
import { VmLifecycleState, VmAction, type ProvisionResponse } from '@capsem/sdk';

const mockFetch = vi.fn();
vi.stubGlobal('fetch', mockFetch);
vi.stubGlobal('WebSocket', class { close() {} });
const api = await import('../api');

function textResponse(body: string, status = 200): Promise<Response> {
  return Promise.resolve(new Response(body, { status }));
}

function jsonResponse(body: unknown, status = 200): Promise<Response> {
  return textResponse(JSON.stringify(body), status);
}

function provision(id: string): ProvisionResponse {
  return { id, name: 'code-dev', profile_id: 'code', status: VmLifecycleState.RUNNING,
    persistent: true, can_resume: false, available_actions: [VmAction.STOP] };
}

// ---- VM lifecycle ----

describe('VM lifecycle', () => {
  beforeEach(async () => {
    mockFetch.mockReset();
    mockFetch
      .mockReturnValueOnce(jsonResponse({ ok: true, version: '1.0.0', service_socket: '/tmp/s' }))
      .mockReturnValueOnce(jsonResponse({ token: 'tok' }))
      .mockReturnValueOnce(jsonResponse({ service: 'running', gateway_version: '1.0.0', vm_count: 0, vms: [], resource_summary: null }));
    await api.init();
  });

  it('provisionVm sends POST /vms/create', async () => {
    mockFetch.mockReturnValueOnce(jsonResponse(provision('vm-1')));
    const result = await api.provisionVm({
      profile_id: 'code',
      name: 'code-dev',
      ram_mb: 2048,
      cpus: 2,
      persistent: true,
    });
    expect(result.id).toBe('vm-1');
    const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
    expect(call[0]).toContain('/vms/create');
    expect(call[1].method).toBe('POST');
    expect(JSON.parse(call[1].body).profile_id).toBe('code');
  });

  it('provisionVm accepts profile-owned resource defaults', async () => {
    mockFetch.mockReturnValueOnce(jsonResponse(provision('code-1')));
    const result = await api.provisionVm({
      profile_id: 'code',
      persistent: true,
    });

    expect(result.id).toBe('code-1');
    const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
    const body = JSON.parse(call[1].body);
    expect(body).toEqual({
      profile_id: 'code',
      persistent: true,
    });
    expect(body).not.toHaveProperty('ram_mb');
    expect(body).not.toHaveProperty('cpus');
  });

  it('refreshes a rotated gateway token and retries VM creation once', async () => {
    mockFetch
      .mockReturnValueOnce(textResponse('{"error":"unauthorized"}', 401))
      .mockReturnValueOnce(jsonResponse({ token: 'fresh-token' }))
      .mockReturnValueOnce(jsonResponse(provision('vm-fresh')));

    const result = await api.provisionVm({
      profile_id: 'code',
      name: 'code-dev',
      ram_mb: 2048,
      cpus: 2,
      persistent: true,
    });

    expect(result.id).toBe('vm-fresh');
    const createCalls = mockFetch.mock.calls.filter(call => String(call[0]).includes('/vms/create'));
    expect(createCalls).toHaveLength(2);
    expect(new Headers(createCalls[0][1].headers).get('Authorization')).toBe('Bearer tok');
    expect(new Headers(createCalls[1][1].headers).get('Authorization')).toBe('Bearer fresh-token');
    expect(mockFetch.mock.calls.some(call => String(call[0]).endsWith('/token'))).toBe(true);
  });

  it('runVm sends POST /run', async () => {
    mockFetch.mockReturnValueOnce(jsonResponse(provision('vm-2')));
    const result = await api.runVm({
      profile_id: 'code',
      ram_mb: 4096,
      cpus: 4,
      persistent: true,
    });
    expect(result.id).toBe('vm-2');
  });

  it('stopVm sends POST /vms/{id}/stop', async () => {
    mockFetch.mockReturnValueOnce(jsonResponse({ success: true, persistent: true }));
    expect(await api.stopVm('vm-1')).toEqual({ success: true, persistent: true });
    const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
    expect(call[0]).toContain('/vms/vm-1/stop');
  });

  it('deleteVm sends DELETE /vms/{id}/delete', async () => {
    mockFetch.mockReturnValueOnce(jsonResponse({ success: true }));
    await api.deleteVm('vm-1');
    const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
    expect(call[0]).toContain('/vms/vm-1/delete');
    expect(call[1].method).toBe('DELETE');
  });

  it('suspendVm sends POST', async () => {
    mockFetch.mockReturnValueOnce(jsonResponse({ success: true }));
    await api.suspendVm('vm-1');
    const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
    expect(call[0]).toContain('/vms/vm-1/pause');
  });

  it('resumeVm sends POST', async () => {
    mockFetch.mockReturnValueOnce(jsonResponse(provision('11111111-1111-4111-8111-111111111111')));
    await api.resumeVm('11111111-1111-4111-8111-111111111111');
    const call = mockFetch.mock.calls[mockFetch.mock.calls.length - 1];
    expect(call[0]).toContain('/vms/11111111-1111-4111-8111-111111111111/resume');
  });

  it('forkVm sends POST with body', async () => {
    mockFetch.mockReturnValueOnce(jsonResponse({ id: 'fork-id', name: 'fork-1', size_bytes: 1024 }));
    const result = await api.forkVm('vm-1', { name: 'fork-1' });
    expect(result.id).toBe('fork-id');
    expect(result.name).toBe('fork-1');
    expect(result.size_bytes).toBe(1024);
  });

  it.each([401, 429])('auth rejection %s refreshes once and preserves the second error', async status => {
    mockFetch.mockReturnValueOnce(textResponse('expired', status))
      .mockReturnValueOnce(jsonResponse({ token: 'replacement' }))
      .mockReturnValueOnce(textResponse('still rejected', status));
    await expect(api.stopVm('vm-1')).rejects.toMatchObject({ name: 'ApiError', status, body: 'still rejected' });
    expect(mockFetch.mock.calls.slice(3).map(call => new URL(call[0]).pathname))
      .toEqual(['/vms/vm-1/stop', '/token', '/vms/vm-1/stop']);
  });

  it('preserves the operation error when token refresh fails', async () => {
    mockFetch.mockReturnValueOnce(textResponse('expired', 401))
      .mockReturnValueOnce(textResponse('token denied', 403));
    await expect(api.stopVm('vm-1')).rejects.toMatchObject({ status: 401, body: 'expired' });
    expect(mockFetch).toHaveBeenCalledTimes(5);
  });

  it('does not replay an operation after a server or network failure', async () => {
    mockFetch.mockReturnValueOnce(textResponse('save failed', 500));
    await expect(api.suspendVm('vm-1')).rejects.toMatchObject({ status: 500, body: 'save failed' });
    mockFetch.mockRejectedValueOnce(new TypeError('connection lost'));
    await expect(api.suspendVm('vm-1')).rejects.toThrow('connection lost');
    expect(mockFetch).toHaveBeenCalledTimes(5);
  });

  it('rejects malformed successful responses without retrying or changing connectivity', async () => {
    mockFetch.mockReturnValueOnce(jsonResponse({ success: true }));
    await expect(api.stopVm('vm-1')).rejects.toThrow();
    expect(api.isConnected()).toBe(true);
    expect(mockFetch).toHaveBeenCalledTimes(4);
  });

  it('rejects unknown lifecycle enums in successful create responses', async () => {
    mockFetch.mockReturnValueOnce(jsonResponse({ ...provision('vm'), status: 'StartingEventually' }));
    await expect(api.provisionVm({ profile_id: 'code' })).rejects.toThrow();
    expect(mockFetch).toHaveBeenCalledTimes(4);
  });

  it('rejects calls without a token before sending a mutation', async () => {
    mockFetch.mockRejectedValueOnce(new TypeError('offline'));
    await api.init();
    await expect(api.stopVm('vm-1')).rejects.toThrow('Gateway not connected');
    expect(mockFetch).toHaveBeenCalledTimes(4);
  });
});
