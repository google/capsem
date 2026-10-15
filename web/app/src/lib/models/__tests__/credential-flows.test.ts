import {expect, it, vi} from 'vitest';
import {ConnectionFlow, SessionGrantFlow, availableCapabilities, capabilityLabel} from '../credential-flows';
import type {CredentialFlowApi} from '../credential-flows';
import type {ConnectionListResponse, SessionGrantListResponse} from '@capsem/sdk';

const catalog = (): ConnectionListResponse => ({connections: [{id: 'connection', scope_profile_id: 'native',
  state: 'connected', available_capabilities: ['agy'], granted_scopes: ['openid'], account_label: 'Account',
  provider: 'google', revision: 1, created_at: '', updated_at: '', last_refresh_at: null,
  provider_revocation: 'not_started', error_code: null}], scope_profiles: [{id: 'native', label: 'AGY',
    application_label: 'Google Antigravity', enrollment_kind: 'native_consumer_sign_in', enabled: true,
    requested_scopes: ['openid'], capabilities: [
      {id: 'agy', label: 'AGY', consumer: 'agy', required_scopes: ['openid'], supported: true},
      {id: 'unsupported', label: 'Other', consumer: 'other', required_scopes: [], supported: false},
    ]}], next_cursor: null} as ConnectionListResponse);
const grants = (): SessionGrantListResponse => ({session_id: 'session', owner_generation: 'owner',
  grants: [], denied_capability_ids: []});
const transaction = {id: 'transaction', state: 'pending', connection_id: null,
  expires_at: '2026-10-14T12:00:00Z', error_code: null};
function fixture() {
  const api = {
    listConnections: vi.fn().mockImplementation(async () => catalog()),
    authorizeGoogleConnection: vi.fn().mockResolvedValue({transaction, authorization_url: 'https://accounts.google.com/o/oauth2/v2/auth'}),
    getConnectionAuthorization: vi.fn().mockResolvedValue(transaction),
    cancelConnectionAuthorization: vi.fn().mockResolvedValue({...transaction, state: 'cancelled'}),
    getConnection: vi.fn().mockImplementation(async () => catalog().connections[0]),
    disconnectConnection: vi.fn().mockImplementation(async () => ({connection: {...catalog().connections[0], state: 'disconnecting', provider_revocation: 'pending'}, local_authority: 'revoked', revoked_grant_count: 1})),
    listSessionCredentialGrants: vi.fn().mockImplementation(async () => grants()),
    createSessionCredentialGrant: vi.fn().mockResolvedValue({id: 'grant', session_id: 'session', owner_generation: 'owner', connection_id: 'connection', capability_ids: ['agy'], state: 'active', runtime_bound: true}),
    revokeSessionCredentialGrant: vi.fn().mockResolvedValue({grant: {id: 'grant', state: 'revoked', session_id: 'session'}, local_authority: 'revoked'}),
  };
  return {api, connection: new ConnectionFlow(api as CredentialFlowApi, () => {}, () => 'request'),
    session: new SessionGrantFlow('session', api as CredentialFlowApi, () => {}, () => 'request')};
}

it('requires a supported enabled catalog profile and never grants a session during consent', async () => {
  const f = fixture();
  const disabled = catalog();
  disabled.scope_profiles[0]!.enabled = false;
  f.api.listConnections.mockResolvedValueOnce(disabled);
  await f.connection.load();
  await f.connection.authorize('native');
  expect(f.api.authorizeGoogleConnection).not.toHaveBeenCalled();
  await f.connection.load();
  await f.connection.authorize('native');
  expect(f.api.authorizeGoogleConnection).toHaveBeenCalledWith({request_id: 'request', scope_profile_id: 'native', reconnect_connection_id: null});
  expect(f.connection.state.url).toBe('https://accounts.google.com/o/oauth2/v2/auth');
  expect(f.api.createSessionCredentialGrant).not.toHaveBeenCalled();
  await f.connection.poll();
  await f.connection.cancel();
  expect(f.api.getConnectionAuthorization).toHaveBeenCalledWith('transaction');
  expect(f.api.cancelConnectionAuthorization).toHaveBeenCalledWith('transaction');
  expect(f.connection.state.url).toBeNull();
});

it('retains a failed explicit consent request ID and never retries automatically', async () => {
  const f = fixture();
  await f.connection.load();
  f.api.authorizeGoogleConnection.mockRejectedValueOnce(new Error('secret-token'));
  await f.connection.authorize('native');
  expect(f.api.authorizeGoogleConnection).toHaveBeenCalledTimes(1);
  expect(f.connection.state.error).not.toContain('secret-token');
  await f.connection.authorize('native');
  expect(f.api.authorizeGoogleConnection.mock.calls[0]).toEqual(f.api.authorizeGoogleConnection.mock.calls[1]);
});

it.each(['javascript:alert(1)', 'https://evil.test/', 'https://accounts.google.com.evil.test/', 'https://user@accounts.google.com/', 'https://accounts.google.com:8443/'])('refuses unsafe consent URL %s while retaining cancellation', async url => {
  const f = fixture();
  await f.connection.load();
  f.api.authorizeGoogleConnection.mockResolvedValueOnce({transaction, authorization_url: url});
  await f.connection.authorize('native');
  expect(f.connection.state.url).toBeNull();
  expect(f.connection.state.error).not.toBeNull();
  await f.connection.cancel();
  expect(f.api.cancelConnectionAuthorization).toHaveBeenCalledWith('transaction');
});

it('shows local grant retirement separately from pending provider revocation', async () => {
  const f = fixture();
  await f.connection.load();
  await f.connection.disconnect('connection');
  expect(f.connection.state.catalog?.connections[0]?.state).toBe('disconnecting');
  expect(f.connection.state.catalog?.connections[0]?.provider_revocation).toBe('pending');
  await f.connection.check('connection');
  expect(f.api.getConnection).toHaveBeenCalledWith('connection');
});

it('filters capability availability through connection, profile support and current session denial', () => {
  const c = catalog();
  expect(availableCapabilities(c, 'connection', [])).toEqual([c.scope_profiles[0]!.capabilities[0]]);
  expect(availableCapabilities(c, 'connection', ['agy'])).toEqual([]);
  c.connections[0]!.state = 'disconnected' as typeof c.connections[0]['state'];
  expect(availableCapabilities(c, 'connection', [])).toEqual([]);
});

it('uses the granting connection profile when capability IDs are reused', () => {
  const c = catalog();
  c.scope_profiles.unshift({...c.scope_profiles[0]!, id: 'other', capabilities: [{...c.scope_profiles[0]!.capabilities[0]!, label: 'Wrong account capability'}]});
  expect(capabilityLabel(c, 'connection', 'agy')).toBe('AGY');
  expect(capabilityLabel(c, 'unknown', 'agy')).toBe('agy');
});

it('uses the service owner, stable retries and only explicitly selected session capabilities', async () => {
  const f = fixture();
  await f.session.load();
  await f.session.grant('connection', ['unsupported']);
  expect(f.api.createSessionCredentialGrant).not.toHaveBeenCalled();
  f.api.createSessionCredentialGrant.mockRejectedValueOnce(new Error('raw-secret'));
  await f.session.grant('connection', ['agy']);
  expect(f.api.createSessionCredentialGrant).toHaveBeenCalledTimes(1);
  expect(f.session.state.error).not.toContain('raw-secret');
  await f.session.grant('connection', ['agy']);
  expect(f.api.createSessionCredentialGrant.mock.calls[0]).toEqual(f.api.createSessionCredentialGrant.mock.calls[1]);
  expect(f.api.createSessionCredentialGrant).toHaveBeenLastCalledWith('session', {
    request_id: 'request', expected_owner_generation: 'owner', connection_id: 'connection', capability_ids: ['agy'], expires_at: null,
  });
  expect(f.api.authorizeGoogleConnection).not.toHaveBeenCalled();
  await f.session.revoke('grant');
  expect(f.api.revokeSessionCredentialGrant).toHaveBeenCalledWith('session', 'grant');
});

it('does not publish late results after the original view closes', async () => {
  const f = fixture();
  let complete!: (value: ConnectionListResponse) => void;
  f.api.listConnections.mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  const pending = f.connection.load();
  f.connection.dispose();
  complete(catalog());
  await pending;
  expect(f.connection.state.catalog).toBeNull();
  await f.connection.authorize('native');
  expect(f.api.authorizeGoogleConnection).not.toHaveBeenCalled();
});

it('creates a fresh explicit request when the service reports a replacement owner', async () => {
  const f = fixture();
  let nonce = 0;
  const flow = new SessionGrantFlow('session', f.api as CredentialFlowApi, () => {}, () => `request-${++nonce}`);
  await flow.load();
  f.api.createSessionCredentialGrant.mockRejectedValueOnce(new Error('unknown outcome'));
  await flow.grant('connection', ['agy']);
  f.api.listSessionCredentialGrants.mockResolvedValueOnce({...grants(), owner_generation: 'replacement'});
  await flow.load();
  await flow.grant('connection', ['agy']);
  expect(f.api.createSessionCredentialGrant.mock.calls.map(([, body]) => [body.request_id, body.expected_owner_generation]))
    .toEqual([['request-1', 'owner'], ['request-2', 'replacement']]);
});

it('refuses a second grant mutation while the original request is pending', async () => {
  const f = fixture();
  await f.session.load();
  let reject!: (reason: Error) => void;
  f.api.createSessionCredentialGrant.mockImplementationOnce(() => new Promise((_, failure) => { reject = failure; }));
  const first = f.session.grant('connection', ['agy']);
  await f.session.grant('connection', ['agy']);
  expect(f.api.createSessionCredentialGrant).toHaveBeenCalledTimes(1);
  reject(new Error('refused'));
  await first;
});

it('allows retained approval inspection and revocation without allowing new grants', async () => {
  const f = fixture();
  const flow = new SessionGrantFlow('session', f.api as CredentialFlowApi, () => {}, () => 'request', false);
  await flow.load();
  expect(f.api.listConnections).not.toHaveBeenCalled();
  await flow.grant('connection', ['agy']);
  expect(f.api.createSessionCredentialGrant).not.toHaveBeenCalled();
  f.api.listSessionCredentialGrants.mockResolvedValueOnce({...grants(), grants: [{id: 'grant', state: 'active', runtime_bound: false}]});
  await flow.load();
  await flow.revoke('grant');
  expect(f.api.revokeSessionCredentialGrant).toHaveBeenCalledWith('session', 'grant');
});
