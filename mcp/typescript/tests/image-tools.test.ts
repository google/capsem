import {createServer as httpServer} from 'node:http';
import {once} from 'node:events';
import {buffer} from 'node:stream/consumers';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {InMemoryTransport} from '@modelcontextprotocol/sdk/inMemory.js';
import {expect, it} from 'vitest';
import {createServer} from '../src/server.js';

const pin = `registry.example/code@sha256:${'a'.repeat(64)}`;
const catalog = {images: [
  {name: 'code', description: 'Tools', architectures: ['amd64'], image: pin, cached: 'unknown'},
  {name: 'other', description: 'Other runtime', architectures: ['arm64'], image: null, cached: 'unknown'},
]};
const pulled = {image: 'code', resolved: pin, digest: `sha256:${'b'.repeat(64)}`};

interface RequestRecord {method: string | undefined; url: string | undefined; body: string; authorization: string | undefined}
interface FixtureReply {status?: number; body?: object | string; hold?: {started: () => void; closed: () => void}}
async function withGateway(run: (client: Client, requests: RequestRecord[]) => Promise<void>, reply: FixtureReply = {}, token = 'client-token'): Promise<void> {
  const requests: RequestRecord[] = [];
  const gateway = httpServer((request, response) => {
    void buffer(request).then(body => {
      requests.push({method: request.method, url: request.url, body: body.toString(), authorization: request.headers.authorization});
      if (reply.hold) {
        response.once('close', reply.hold.closed);
        reply.hold.started();
        return;
      }
      response.setHeader('Content-Type', 'application/json');
      response.statusCode = reply.status ?? 200;
      response.end(typeof reply.body === 'string' ? reply.body : JSON.stringify(reply.body ?? (request.method === 'GET' ? catalog : pulled)));
    }).catch((error: unknown) => response.destroy(error instanceof Error ? error : new Error(String(error))));
  });
  gateway.listen(0, '127.0.0.1');
  await once(gateway, 'listening');
  const address = gateway.address();
  if (!address || typeof address === 'string') throw new Error('missing HTTP address');
  const server = createServer({gatewayUrl: `http://127.0.0.1:${address.port}`, token, timeoutMs: 5000});
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
  const client = new Client({name: 'image-tools-test', version: '1'});
  try {
    await server.connect(serverTransport);
    await client.connect(clientTransport);
    await run(client, requests);
  } finally {
    await client.close(); await server.close();
    gateway.closeAllConnections();
    await new Promise<void>((resolve, reject) => gateway.close(error => error ? reject(error) : resolve()));
  }
}

it('offers image catalog and prefetch over authenticated MCP and gateway HTTP', async () => {
  await withGateway(async (client, requests) => {
    const tools = (await client.listTools()).tools;
    expect(tools.map(tool => tool.name)).toContain('capsem_image_list');
    expect(tools.map(tool => tool.name)).toContain('capsem_image_pull');
    const listed = await client.callTool({name: 'capsem_image_list', arguments: {refresh: true}});
    expect(listed.isError).not.toBe(true);
    expect(listed.structuredContent).toEqual(catalog);
    const result = await client.callTool({name: 'capsem_image_pull', arguments: {image: 'code'}});
    expect(result.isError).not.toBe(true);
    expect(result.structuredContent).toEqual(pulled);
    expect(requests).toEqual([
      {method: 'GET', url: '/images?refresh=true', body: '', authorization: 'Bearer client-token'},
      {method: 'POST', url: '/images/pull', body: '{"image":"code"}', authorization: 'Bearer client-token'},
    ]);
    expect(JSON.stringify(tools.find(tool => tool.name === 'capsem_image_pull')?.inputSchema)).not.toMatch(/password|token|registry|ca_pem/);
  });
});

it('refuses malformed or credential-bearing tool arguments before HTTP', async () => {
  await withGateway(async (client, requests) => {
    for (const call of [
      {name: 'capsem_image_pull', arguments: {image: ''}},
      {name: 'capsem_image_pull', arguments: {image: '  '}},
      {name: 'capsem_image_pull', arguments: {image: 'code', registry: {password: 'argument-secret'}}},
      {name: 'capsem_image_list', arguments: {refresh: 'yes'}},
    ]) {
      const result = await client.callTool(call);
      expect(result.isError).toBe(true);
      expect(JSON.stringify(result)).not.toContain('argument-secret');
    }
    expect(requests).toHaveLength(0);
  });
});

it('preserves service refusal status without leaking raw causes or replaying', async () => {
  await withGateway(async (client, requests) => {
    const result = await client.callTool({name: 'capsem_image_pull', arguments: {image: 'code'}});
    expect(result.isError).toBe(true);
    expect(result.structuredContent).toEqual({error: {kind: 'http', status: 403}});
    expect(JSON.stringify(result)).not.toMatch(/client-token|registry-secret/);
    expect(requests).toHaveLength(1);
  }, {status: 403, body: 'denied client-token registry-secret'});
});

it('rejects malformed image catalog responses without claiming cache readiness', async () => {
  await withGateway(async (client, requests) => {
    const result = await client.callTool({name: 'capsem_image_list', arguments: {}});
    expect(result.isError).toBe(true);
    expect(result.structuredContent).toEqual({error: {kind: 'internal'}});
    expect(requests).toHaveLength(1);
  }, {body: {images: [{...catalog.images[0], cached: 'available'}]}});
});

it('keeps gateway bearer ownership separate for each MCP server', async () => {
  await withGateway(async (first, firstRequests) => {
    await withGateway(async (second, secondRequests) => {
      expect((await first.callTool({name: 'capsem_image_list', arguments: {}})).isError).not.toBe(true);
      expect((await second.callTool({name: 'capsem_image_list', arguments: {}})).isError).not.toBe(true);
      expect(firstRequests[0]?.authorization).toBe('Bearer first-token');
      expect(secondRequests[0]?.authorization).toBe('Bearer second-token');
    }, {}, 'second-token');
  }, {}, 'first-token');
});

it('closes the local prefetch HTTP request when its MCP caller cancels', async () => {
  let markStarted!: () => void;
  let markClosed!: () => void;
  const started = new Promise<void>(resolve => { markStarted = resolve; });
  const closed = new Promise<void>(resolve => { markClosed = resolve; });
  await withGateway(async (client, requests) => {
    const controller = new AbortController();
    const pending = client.callTool({name: 'capsem_image_pull', arguments: {image: 'code'}}, undefined, {signal: controller.signal});
    await started;
    controller.abort();
    await expect(pending).rejects.toThrow(/AbortError/);
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      await Promise.race([closed, new Promise<never>((_resolve, reject) => {
        timer = setTimeout(() => reject(new Error('cancelled prefetch retained its HTTP connection')), 500);
      })]);
    } finally { clearTimeout(timer); }
    expect(requests).toHaveLength(1);
  }, {hold: {started: markStarted, closed: markClosed}});
});
