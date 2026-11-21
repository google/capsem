import {expect, it} from 'vitest';
import {Hypervisor, HttpError, RequestApplicationState, RequestState, type EnvironmentCreateRequest, type CallOptions} from '../src/index.js';
import {gateway} from './gateway.js';

const requestId = '00000000-0000-4000-8000-000000000001';
const environmentId = '00000000-0000-4000-8000-000000000002';
const input: EnvironmentCreateRequest = {request_id: requestId, specification: {name: 'development'}, reason: 'prepare blueprint'};
const pending = {request_id: requestId, environment_id: environmentId, decision: 'approved', application: 'pending', result: null, application_error: null};
const current = {document: {identity: {schema_version: 1, id: environmentId, revision_id: requestId}, specification: {name: 'development'}}, timestamp_unix_ms: 1, actor: 'operator', reason: 'approved', request_id: requestId};

it('preserves approval versus application and explicitly reconciles the same request ID', async () => {
  await gateway((request, response) => response.end(JSON.stringify(request.method === 'POST' ? pending : current)), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      const created = await hv.environments.create(input);
      expect(created.request_id).toBe(requestId);
      expect(created.decision).toBe(RequestState.APPROVED);
      expect(created.application).toBe(RequestApplicationState.PENDING);
      expect(created.result).toBeNull();
      const inspected = await hv.environments.info(environmentId);
      expect(inspected.document.identity.id).toBe(environmentId);
      expect(inspected.document.specification.name).toBe('development');
      expect(inspected.actor).toBe('operator');
      expect(await hv.environments.create(input)).toEqual(created);
      expect(received.map(request => [request.method, request.url])).toEqual([
        ['POST', '/environments/create'], ['GET', `/environments/${environmentId}/info`], ['POST', '/environments/create'],
      ]);
      for (const request of received) expect(request.headers.authorization).toBe('Bearer token');
      for (const request of received.filter(request => request.method === 'POST')) {
        expect(JSON.parse(request.body.toString())).toEqual(input);
      }
      hv.close();
      await expect(hv.environments.create(input)).rejects.toThrow('closed');
      expect(received).toHaveLength(3);
    } finally { hv.close(); }
  });
});

it('retains conflicts without mutation replay or implicit refresh', async () => {
  await gateway((_request, response) => response.writeHead(409).end('changed intent'), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      const failure = hv.environments.create(input);
      await expect(failure).rejects.toBeInstanceOf(HttpError);
      await expect(failure).rejects.toMatchObject({status: 409});
      expect(received).toHaveLength(1);
    } finally { hv.close(); }
  });
});

it('refuses a substituted request identity', async () => {
  await gateway((_request, response) => response.end(JSON.stringify({...pending, request_id: environmentId})), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      await expect(hv.environments.create(input)).rejects.toThrow('identity changed');
      expect(received).toHaveLength(1);
    } finally { hv.close(); }
  });
});

it('refuses a substituted environment identity', async () => {
  await gateway((_request, response) => response.end(JSON.stringify(current)), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      await expect(hv.environments.info(requestId)).rejects.toThrow('identity changed');
      expect(received).toHaveLength(1);
    } finally { hv.close(); }
  });
});

it('reads review provenance and refuses a changed request identity', async () => {
  const summary = {
    request_id: requestId, kind: 'environment_change', environment_id: environmentId, actor: 'operator',
    reason: 'test review', timestamp_unix_ms: 1, expires_at_unix_ms: 1000, decision: 'pending', application: 'pending',
  };
  const detail = {
    summary, observed_revision: '0123456789ab',
    proposed: {availability: 'complete', purged_variables: [], specification: {name: 'development', variables: {EDITOR: 'emacs'}}},
    targets: [],
  };
  await gateway((request, response) => response.end(JSON.stringify(
    request.url?.startsWith('/requests/list') ? {requests: [summary], next_cursor: 9} : detail,
  )), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      expect(await hv.requests.list({after: 7, limit: 1, pendingOnly: true})).toMatchObject({next_cursor: 9});
      expect(await hv.requests.info(requestId)).toEqual(detail);
      await expect(hv.requests.info(environmentId)).rejects.toThrow('identity changed');
      expect(received.map(request => request.url)).toEqual([
        '/requests/list?after=7&limit=1&pending_only=true', `/requests/${requestId}/info`, `/requests/${environmentId}/info`,
      ]);
    } finally { hv.close(); }
  });
});

it.each(['create', 'change'] as const)('retains the %s budget, deadlines and cancellation without replay', async mutation => {
  await gateway((_request, response) => {setTimeout(() => response.end(JSON.stringify(pending)), 150);}, async (url, received) => {
    const hv = new Hypervisor(url, 'token', {timeoutMs: 30});
    try {
      const invoke = (options: CallOptions = {}) => mutation === 'create' ? hv.environments.create(input, options) : hv.environments.change(environmentId, {...input, base_revision_id: requestId}, options);
      expect((await invoke()).request_id).toBe(requestId);
      await expect(invoke({timeoutMs: 30})).rejects.toMatchObject({name: 'TimeoutError'});
      const controller = new AbortController();
      const rejected = expect(invoke({signal: controller.signal})).rejects.toMatchObject({name: 'AbortError'});
      setTimeout(() => controller.abort(), 30);
      await rejected;
      expect(received.map(request => request.url)).toEqual(Array<string>(3).fill(mutation === 'create' ? '/environments/create' : `/environments/${environmentId}/changes`));
    } finally { hv.close(); }
  });
});


it.each([
  {response: pending, valid: true},
  {response: {...pending, request_id: environmentId}, valid: false},
  {response: {...pending, environment_id: requestId}, valid: false},
])('preserves change base and rejects substituted identities ($valid)', async ({response, valid}) => {
  await gateway((_request, reply) => reply.end(JSON.stringify(response)), async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    try {
      const base = '00000000-0000-4000-8000-000000000003';
      const request = {...input, base_revision_id: base};
      const result = hv.environments.change(environmentId, request);
      if (valid) expect((await result).application).toBe(RequestApplicationState.PENDING);
      else await expect(result).rejects.toThrow('identity changed');
      expect(received).toHaveLength(1);
      expect(received[0]?.url).toBe(`/environments/${environmentId}/changes`);
      expect(received[0]?.headers.authorization).toBe('Bearer token');
      expect(JSON.parse(received[0]?.body.toString() ?? '')).toEqual(request);
    } finally { hv.close(); }
  });
});
