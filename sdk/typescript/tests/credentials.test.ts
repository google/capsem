import {expect, it} from 'vitest';
import {Hypervisor, HttpError} from '../src/index.js';
import {gateway} from './gateway.js';

it('injects exact file or memory requests and returns an opaque reference', async () => {
  const reference = `credential:blake3:${'a'.repeat(64)}`;
  for (const storage of ['file', 'memory'] as const) {
    await gateway((_request, reply) => reply.end(JSON.stringify({credential_ref: reference, storage})), async (url, received) => {
      const hv = new Hypervisor(url, 'gateway-token');
      try {
        const response = await hv.credentials.inject('openai', 'private-test-key', {storage});
        expect(response).toEqual({credential_ref: reference, storage});
        expect(received[0]?.method).toBe('POST');
        expect(received[0]?.url).toBe('/credentials/inject');
        expect(received[0]?.headers.authorization).toBe('Bearer gateway-token');
        expect(JSON.parse(received[0]?.body.toString() ?? '')).toEqual({provider: 'openai', value: 'private-test-key', storage});
        expect(received).toHaveLength(1);
      } finally { hv.close(); }
    });
  }
});

it('does not replay refused credential injections', async () => {
  await gateway((_request, reply) => { reply.writeHead(503); reply.end('{"error":"credential handoff unavailable"}'); }, async (url, received) => {
    const hv = new Hypervisor(url, 'gateway-token');
    try {
      await expect(hv.credentials.inject('google', 'private-test-key', {storage: 'memory'})).rejects.toBeInstanceOf(HttpError);
      expect(received).toHaveLength(1);
    } finally { hv.close(); }
  });
});
