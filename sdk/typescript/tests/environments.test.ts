import {expect, it} from 'vitest';
import {Hypervisor, HttpError, RequestApplicationState, RequestState, type EnvironmentCreateRequest} from '../src/index.js';
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

it('retains the creation budget, explicit deadlines and cancellation without replay', async () => {
  await gateway((_request, response) => {setTimeout(() => response.end(JSON.stringify(pending)), 150);}, async (url, received) => {
    const hv = new Hypervisor(url, 'token', {timeoutMs: 30});
    try {
      expect((await hv.environments.create(input)).request_id).toBe(requestId);
      await expect(hv.environments.create(input, {timeoutMs: 30})).rejects.toMatchObject({name: 'TimeoutError'});
      const controller = new AbortController();
      const rejected = expect(hv.environments.create(input, {signal: controller.signal})).rejects.toMatchObject({name: 'AbortError'});
      setTimeout(() => controller.abort(), 30);
      await rejected;
      expect(received.map(request => request.url)).toEqual(Array<string>(3).fill('/environments/create'));
    } finally { hv.close(); }
  });
});
