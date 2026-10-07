import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockFetch = vi.fn();
vi.stubGlobal('fetch', mockFetch);
vi.stubGlobal('WebSocket', class { close() {} });
const api = await import('../api');
const json = (value: unknown) => Promise.resolve(new Response(JSON.stringify(value)));

describe('typed image catalog', () => {
  beforeEach(async () => {
    mockFetch.mockReset();
    mockFetch.mockReturnValueOnce(json({ ok: true, version: '1', service_socket: '/tmp/s' }))
      .mockReturnValueOnce(json({ token: 'image-test-token' }))
      .mockReturnValueOnce(json({ service: 'running', gateway_version: '1', vm_count: 0, vms: [], resource_summary: null }));
    await api.init();
  });

  it('rejects incomplete rows instead of presenting them as catalog choices', async () => {
    mockFetch.mockReturnValueOnce(json({ images: [{ name: 'code' }] }));
    await expect(api.getImages()).rejects.toThrow();
  });

  it('retains compatibility, pin, description and cache truth', async () => {
    const response = { images: [
      { name: 'code', description: 'Development tools', architectures: ['amd64'], cached: 'unknown', image: `registry.example/code@sha256:${'a'.repeat(64)}` },
      { name: 'other', description: 'Other architecture', architectures: ['arm64'], cached: 'unknown', image: null },
    ] };
    mockFetch.mockReturnValueOnce(json(response));
    expect(await api.getImages()).toEqual(response);
    const request = mockFetch.mock.calls.at(-1)!;
    expect(new URL(request[0]).pathname).toBe('/images');
    expect(new Headers(request[1].headers).get('Authorization')).toBe('Bearer image-test-token');
  });
});
