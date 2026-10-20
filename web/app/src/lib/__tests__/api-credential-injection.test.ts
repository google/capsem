import {beforeEach, expect, it, vi} from 'vitest';
import {CredentialInjectProvider, CredentialStorage} from '@capsem/sdk';

const mockFetch = vi.fn<typeof fetch>();
vi.stubGlobal('fetch', mockFetch);
vi.stubGlobal('WebSocket', class { close() {} });
const api = await import('../api');
const reference = `credential:blake3:${'a'.repeat(64)}`;
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), {status});

beforeEach(async () => {
  mockFetch.mockReset();
  mockFetch.mockResolvedValueOnce(json({ok: true, version: '1', service_socket: '/tmp/s'}))
    .mockResolvedValueOnce(json({token: 'fixture-token'}))
    .mockResolvedValueOnce(json({service: 'running', gateway_version: '1', vm_count: 0, vms: []}));
  await api.init();
  mockFetch.mockClear();
});

it.each([CredentialStorage.FILE, CredentialStorage.MEMORY])('injects explicit %s material through authenticated typed HTTP', async storage => {
  const response = {credential_ref: reference, storage};
  mockFetch.mockResolvedValueOnce(json(response));
  expect(await api.injectCredential(CredentialInjectProvider.OPENAI, 'fixture-secret', storage)).toEqual(response);
  expect(mockFetch).toHaveBeenCalledTimes(1);
  const [url, request] = mockFetch.mock.calls[0]!;
  expect(url).toBe(api.getBaseUrl() + '/credentials/inject');
  expect(new Headers(request?.headers).get('Authorization')).toBe('Bearer fixture-token');
  expect(request?.method).toBe('POST');
  expect(request?.redirect).toBe('manual');
  expect(JSON.parse(String(request?.body))).toEqual({provider: 'openai', value: 'fixture-secret', storage});
});

it.each([401, 429, 503])('never refreshes/replays an injection after %s', async status => {
  mockFetch.mockResolvedValueOnce(json({error: 'fixture denial'}, status));
  await expect(api.injectCredential(CredentialInjectProvider.OPENAI, 'fixture-secret', CredentialStorage.MEMORY)).rejects.toMatchObject({status});
  expect(mockFetch).toHaveBeenCalledTimes(1);
});

it('does not report mock success while disconnected', async () => {
  mockFetch.mockRejectedValueOnce(new TypeError('offline'));
  await api.init();
  mockFetch.mockClear();
  await expect(api.injectCredential(CredentialInjectProvider.OPENAI, 'fixture-secret', CredentialStorage.FILE)).rejects.toThrow();
  expect(mockFetch).not.toHaveBeenCalled();
});
