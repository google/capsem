import {beforeEach, expect, it, vi} from 'vitest';
import type {AuthorizeGoogleRequest, CreateSessionGrantRequest} from '@capsem/sdk';

const mockFetch = vi.fn<typeof fetch>();
vi.stubGlobal('fetch', mockFetch);
vi.stubGlobal('WebSocket', class { close() {} });
const api = await import('../api');
const session = 'f324d12d-86af-448a-9f99-dd1fcceae68c';
const owner = 'bb120782-2272-4b3d-a2e6-a26ee2f8ab61';
const requestId = 'fc120782-2272-4b3d-a2e6-a26ee2f8ab61';
const connectionId = 'connection /?&é', transactionId = 'transaction /?&é', grantId = 'grant /?&é';
const instant = '2026-10-14T00:00:00Z';
const connection = {id: connectionId, provider: 'google', state: 'connected', revision: 1,
  scope_profile_id: 'agy-native', account_label: null, granted_scopes: ['openid'], available_capabilities: ['agy'],
  created_at: instant, updated_at: instant, last_refresh_at: null, provider_revocation: 'not_started', error_code: null};
const transaction = {id: transactionId, state: 'pending', connection_id: null, error_code: null, expires_at: instant};
const grant = {id: grantId, session_id: session, owner_generation: owner, connection_id: connectionId,
  capability_ids: ['agy'], state: 'active', revision: 1, runtime_bound: true,
  created_at: instant, expires_at: instant, revoked_at: null};
const authorize: AuthorizeGoogleRequest = {request_id: requestId, scope_profile_id: 'agy-native', reconnect_connection_id: null};
const create: CreateSessionGrantRequest = {request_id: requestId, expected_owner_generation: owner,
  connection_id: connectionId, capability_ids: ['agy'], expires_at: null};
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), {status});
const encoded = (id: string) => encodeURIComponent(id).replaceAll('.', '%2E');
const calls = [
  {method: 'GET', path: '/connections', call: () => api.listConnections(),
    response: {connections: [connection], scope_profiles: [], next_cursor: null}},
  {method: 'POST', path: '/connections/google/authorize', call: () => api.authorizeGoogleConnection(authorize), body: authorize,
    response: {transaction, authorization_url: 'https://accounts.google.com/consent'}},
  {method: 'GET', path: `/connections/authorizations/${encoded(transactionId)}`, call: () => api.getConnectionAuthorization(transactionId), response: transaction},
  {method: 'DELETE', path: `/connections/authorizations/${encoded(transactionId)}`, call: () => api.cancelConnectionAuthorization(transactionId), response: transaction},
  {method: 'GET', path: `/connections/${encoded(connectionId)}`, call: () => api.getConnection(connectionId), response: connection},
  {method: 'DELETE', path: `/connections/${encoded(connectionId)}`, call: () => api.disconnectConnection(connectionId),
    response: {connection, local_authority: 'revoked', revoked_grant_count: 1}},
  {method: 'GET', path: `/vms/${session}/credential-grants`, call: () => api.listSessionCredentialGrants(session),
    response: {session_id: session, owner_generation: owner, grants: [grant], denied_capability_ids: []}},
  {method: 'POST', path: `/vms/${session}/credential-grants`, call: () => api.createSessionCredentialGrant(session, create), body: create, response: grant},
  {method: 'DELETE', path: `/vms/${session}/credential-grants/${encoded(grantId)}`, call: () => api.revokeSessionCredentialGrant(session, grantId),
    response: {grant, local_authority: 'revoked'}},
];

beforeEach(async () => {
  mockFetch.mockReset();
  mockFetch.mockResolvedValueOnce(json({ok: true, version: '1', service_socket: '/tmp/s'}))
    .mockResolvedValueOnce(json({token: 'fixture-token'}))
    .mockResolvedValueOnce(json({service: 'running', gateway_version: '1', vm_count: 0, vms: []}));
  await api.init();
  mockFetch.mockClear();
});

it.each(calls)('uses typed authenticated $method $path', async item => {
  mockFetch.mockResolvedValueOnce(json(item.response));
  expect(await item.call()).toEqual(item.response);
  expect(mockFetch).toHaveBeenCalledTimes(1);
  const [url, request] = mockFetch.mock.calls[0]!;
  expect(url).toBe(api.getBaseUrl() + item.path);
  expect(request?.method).toBe(item.method);
  expect(request?.redirect).toBe('manual');
  expect(new Headers(request?.headers).get('Authorization')).toBe('Bearer fixture-token');
  if ('body' in item) expect(JSON.parse(String(request?.body))).toEqual(item.body);
  else expect(request?.body).toBeUndefined();
});

it.each(calls.filter(item => item.method !== 'GET'))('never replays $method $path after refusal', async item => {
  for (const status of [401, 429, 503]) {
    mockFetch.mockClear();
    mockFetch.mockResolvedValueOnce(json({code: 'policy_denied', retryable: false}, status));
    await expect(item.call()).rejects.toMatchObject({status});
    expect(mockFetch).toHaveBeenCalledTimes(1);
  }
});

it('refuses substituted transaction, connection and grant ownership', async () => {
  mockFetch.mockResolvedValueOnce(json({...transaction, id: 'other'}));
  await expect(api.getConnectionAuthorization(transactionId)).rejects.toThrow('changed');
  mockFetch.mockResolvedValueOnce(json({...connection, id: 'other'}));
  await expect(api.getConnection(connectionId)).rejects.toThrow('changed');
  mockFetch.mockResolvedValueOnce(json({...grant, owner_generation: 'other'}));
  await expect(api.createSessionCredentialGrant(session, create)).rejects.toThrow('changed');
  expect(mockFetch).toHaveBeenCalledTimes(3);
});

it('refuses all operations offline without fabricated results', async () => {
  mockFetch.mockRejectedValueOnce(new TypeError('offline'));
  await api.init();
  mockFetch.mockClear();
  for (const item of calls) await expect(item.call()).rejects.toThrow();
  expect(mockFetch).not.toHaveBeenCalled();
});
