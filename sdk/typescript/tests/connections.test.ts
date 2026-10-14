import {expect, it} from 'vitest';
import {Hypervisor, HttpError, type AuthorizeGoogleRequest, type CreateSessionGrantRequest} from '../src/index.js';
import {sample, schemas} from './contract.js';
import {gateway} from './gateway.js';

const OWNER = 'bb120782-2272-4b3d-a2e6-a26ee2f8ab61';
const REQUEST = 'fc120782-2272-4b3d-a2e6-a26ee2f8ab61';
const SESSION = 'session /?&é', CONNECTION = 'connection /?&é';
const TRANSACTION = 'transaction /?&é', GRANT = 'grant /?&é';
const connection = () => ({...sample(schemas.ConnectionSummary ?? {}) as object, id: CONNECTION});
const transaction = () => ({...sample(schemas.AuthorizationTransaction ?? {}) as object, id: TRANSACTION});
const grant = () => ({...sample(schemas.SessionGrantSummary ?? {}) as object,
  id: GRANT, session_id: SESSION, owner_generation: OWNER, connection_id: CONNECTION, capability_ids: ['agy']});
const authorizeRequest: AuthorizeGoogleRequest = {
  request_id: REQUEST, scope_profile_id: 'agy-native', reconnect_connection_id: CONNECTION,
};
const grantRequest: CreateSessionGrantRequest = {
  request_id: REQUEST, expected_owner_generation: OWNER, connection_id: CONNECTION,
  capability_ids: ['agy'], expires_at: null,
};

it.each([
  {call: (hv: Hypervisor) => hv.connections.authorization(TRANSACTION), response: {...transaction(), id: 'other'}},
  {call: (hv: Hypervisor) => hv.connections.cancelAuthorization(TRANSACTION), response: {...transaction(), id: 'other'}},
  {call: (hv: Hypervisor) => hv.connections.get(CONNECTION), response: {...connection(), id: 'other'}},
  {call: (hv: Hypervisor) => hv.connections.disconnect(CONNECTION), response: {connection: {...connection(), id: 'other'}, local_authority: 'revoked', revoked_grant_count: 1}},
  {call: (hv: Hypervisor) => hv.vm({id: SESSION}).credentialGrants.revoke(GRANT), response: {grant: {...grant(), id: 'other'}, local_authority: 'revoked'}},
  {call: (hv: Hypervisor) => hv.vm({id: SESSION}).credentialGrants.revoke(GRANT), response: {grant: {...grant(), session_id: 'other'}, local_authority: 'revoked'}},
  {call: (hv: Hypervisor) => hv.vm({id: SESSION}).credentialGrants.list(), response: {session_id: 'other', owner_generation: OWNER, grants: [], denied_capability_ids: []}},
  {call: (hv: Hypervisor) => hv.vm({id: SESSION}).credentialGrants.list(), response: {session_id: SESSION, owner_generation: 'other', grants: [grant()], denied_capability_ids: []}},
])('rejects substituted identity %#', async ({call, response}) => {
  await gateway((_, reply) => reply.end(JSON.stringify(response)), async (url, received) => {
    const hv = new Hypervisor(url, 'gateway-token');
    try {
      await expect(call(hv)).rejects.toThrow('changed');
      expect(received).toHaveLength(1);
    } finally {hv.close();}
  });
});

it('executes all nine friendly calls as exact authenticated explicit requests', async () => {
  const responses = [
    {connections: [connection()], scope_profiles: [], next_cursor: null},
    {transaction: transaction(), authorization_url: 'https://accounts.google.com/consent'},
    transaction(), transaction(), connection(),
    {connection: connection(), local_authority: 'revoked', revoked_grant_count: 1},
    {session_id: SESSION, owner_generation: OWNER, grants: [grant()], denied_capability_ids: []},
    grant(), {grant: grant(), local_authority: 'revoked'},
  ];
  let index = 0;
  await gateway((_, reply) => {
    reply.statusCode = index === 1 ? 202 : index === 7 ? 201 : 200;
    reply.end(JSON.stringify(responses[index++]));
  }, async (url, received) => {
    const hv = new Hypervisor(url, 'gateway-token');
    try {
      await hv.connections.list();
      await hv.connections.authorizeGoogle(authorizeRequest);
      await hv.connections.authorization(TRANSACTION);
      await hv.connections.cancelAuthorization(TRANSACTION);
      await hv.connections.get(CONNECTION);
      await hv.connections.disconnect(CONNECTION);
      const grants = hv.vm({id: SESSION}).credentialGrants;
      await grants.list();
      await grants.create(grantRequest);
      await grants.revoke(GRANT);
      const encoded = (value: string) => encodeURIComponent(value).replaceAll('.', '%2E');
      expect(received.map(request => [request.method, request.url])).toEqual([
        ['GET', '/connections'], ['POST', '/connections/google/authorize'],
        ['GET', `/connections/authorizations/${encoded(TRANSACTION)}`],
        ['DELETE', `/connections/authorizations/${encoded(TRANSACTION)}`],
        ['GET', `/connections/${encoded(CONNECTION)}`], ['DELETE', `/connections/${encoded(CONNECTION)}`],
        ['GET', `/vms/${encoded(SESSION)}/credential-grants`],
        ['POST', `/vms/${encoded(SESSION)}/credential-grants`],
        ['DELETE', `/vms/${encoded(SESSION)}/credential-grants/${encoded(GRANT)}`],
      ]);
      for (const request of received) expect(request.headers.authorization).toBe('Bearer gateway-token');
      expect(JSON.parse(received[1]?.body.toString() ?? '')).toEqual(authorizeRequest);
      expect(JSON.parse(received[7]?.body.toString() ?? '')).toEqual(grantRequest);
    } finally {hv.close();}
  });
});

it.each([
  {session_id: 'other'}, {owner_generation: 'other'}, {connection_id: 'other'},
  {capability_ids: ['other']}, {capability_ids: ['agy', 'agy']},
])('rejects substituted grant authority %j', async changed => {
  await gateway((_, reply) => reply.end(JSON.stringify({...grant(), ...changed})), async (url, received) => {
    const hv = new Hypervisor(url, 'gateway-token');
    try {
      await expect(hv.vm({id: SESSION}).credentialGrants.create(grantRequest)).rejects.toThrow(/grant.*changed/);
      expect(received).toHaveLength(1);
    } finally {hv.close();}
  });
});

it('does not replay refused consent', async () => {
  await gateway((_, reply) => reply.writeHead(503).end('{"error":"credential request refused","code":"configuration_required"}'), async (url, received) => {
    const hv = new Hypervisor(url, 'gateway-token');
    try {
      await expect(hv.connections.authorizeGoogle(authorizeRequest)).rejects.toBeInstanceOf(HttpError);
      expect(received).toHaveLength(1);
    } finally {hv.close();}
  });
});

it('exposes known credential codes without rejecting future error envelopes', () => {
  expect(new HttpError(503, '{"code":"configuration_required"}').credentialCode).toBe('configuration_required');
  for (const body of ['not-json', 'null', '[]', '{"code":"future_code"}', '{}']) {
    const error = new HttpError(503, body);
    expect(error.credentialCode).toBeUndefined();
    expect(error.status).toBe(503);
    expect(error.body).toBe(body);
  }
});

it('checks grant responses against the original request after caller mutation', async () => {
  const request = {...grantRequest, capability_ids: ['agy']};
  await gateway((_, reply) => {
    request.capability_ids[0] = 'other';
    reply.end(JSON.stringify({...grant(), capability_ids: ['other']}));
  }, async (url, received) => {
    const hv = new Hypervisor(url, 'gateway-token');
    try {
      await expect(hv.vm({id: SESSION}).credentialGrants.create(request)).rejects.toThrow(/grant.*changed/);
      expect(JSON.parse(received[0]?.body.toString() ?? '')).toMatchObject({capability_ids: ['agy']});
      expect(request.capability_ids).toEqual(['other']);
    } finally {hv.close();}
  });
});

it.each(['consent', 'grant', 'name'])('bounds %s requests with the caller deadline', async operation => {
  await gateway(() => {}, async (url, received) => {
    const hv = new Hypervisor(url, 'gateway-token');
    try {
      const options = {timeoutMs: 100};
      const request = operation === 'consent' ? hv.connections.authorizeGoogle(authorizeRequest, options)
        : operation === 'grant' ? hv.vm({id: SESSION}).credentialGrants.create(grantRequest, options)
          : hv.vm({name: 'named'}).credentialGrants.list(options);
      await expect(request).rejects.toMatchObject({name: 'TimeoutError'});
      expect(received).toHaveLength(1);
      if (operation === 'name') expect(received[0]?.url).toBe('/vms/list');
    } finally {hv.close();}
  });
});

it('propagates explicit caller cancellation without replay', async () => {
  const controller = new AbortController();
  let arrived!: () => void;
  const receivedRequest = new Promise<void>(resolve => {arrived = resolve;});
  await gateway(() => arrived(), async (url, received) => {
    const hv = new Hypervisor(url, 'gateway-token');
    try {
      const pending = hv.connections.authorizeGoogle(authorizeRequest, {signal: controller.signal});
      const rejected = expect(pending).rejects.toMatchObject({name: 'AbortError'});
      await receivedRequest;
      controller.abort();
      await rejected;
      expect(received).toHaveLength(1);
    } finally {hv.close();}
  });
});
