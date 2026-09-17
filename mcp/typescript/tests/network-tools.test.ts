import {createServer as createHttpServer} from 'node:http';
import type {AddressInfo} from 'node:net';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {InMemoryTransport} from '@modelcontextprotocol/sdk/inMemory.js';
import {afterEach, beforeEach, describe, expect, it} from 'vitest';
import {createServer} from '../src/server.js';

interface RequestRecord {method: string; url: string; authorization?: string; body: Buffer}
const network = {created_unix_ms: 1, id: 'net-1', members: [], name: 'team', subnet: '10.88.0.0/24'};

describe('network-tools', () => {
  const requests: RequestRecord[] = [];
  let client: Client;
  let closeAll: () => Promise<void>;

  beforeEach(async () => {
    requests.length = 0;
    const gateway = createHttpServer(async (request, response) => {
      const chunks: Buffer[] = [];
      for await (const chunk of request) chunks.push(Buffer.from(chunk));
      const record: RequestRecord = {
        method: request.method ?? '', url: request.url ?? '', body: Buffer.concat(chunks),
        ...(request.headers.authorization === undefined ? {} : {authorization: request.headers.authorization}),
      };
      requests.push(record);
      const path = new URL(record.url, 'http://gateway.test').pathname;
      let result: object;
      if (path === '/networks' && record.method === 'POST') result = network;
      else if (path === '/networks') result = {networks: [network]};
      else if (path === '/networks/net-1/logs') result = {cursor: 'c1', events: [], next_cursor: 'c2'};
      else if (path === '/networks/net-1' && record.method === 'DELETE') result = {success: true};
      else if (path === '/networks/net-1' || path === '/networks/net-1/members/vm-1') result = network;
      else {
        response.writeHead(404).end();
        return;
      }
      response.setHeader('content-type', 'application/json');
      response.end(JSON.stringify(result));
    });
    await new Promise<void>(resolve => gateway.listen(0, '127.0.0.1', resolve));
    const port = (gateway.address() as AddressInfo).port;
    const server = createServer({gatewayUrl: `http://127.0.0.1:${port}`, token: 'secret', timeoutMs: 5_000});
    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    client = new Client({name: 'network-tools-test', version: '1'});
    await server.connect(serverTransport);
    await client.connect(clientTransport);
    closeAll = async () => {
      await client.close();
      await server.close();
      await new Promise<void>((resolve, reject) => gateway.close(error => error ? reject(error) : resolve()));
    };
  });

  afterEach(async () => closeAll());

  it('publishes all typed network operations through authenticated gateway HTTP', async () => {
    const calls: {name: string; arguments: Record<string, unknown>}[] = [
      {name: 'capsem_network_create', arguments: {name: 'team'}},
      {name: 'capsem_network_list', arguments: {}},
      {name: 'capsem_network_inspect', arguments: {network_id: 'net-1'}},
      {name: 'capsem_network_attach', arguments: {network_id: 'net-1', vm_id: 'vm-1'}},
      {name: 'capsem_network_detach', arguments: {network_id: 'net-1', vm_id: 'vm-1'}},
      {name: 'capsem_network_logs', arguments: {
        network_id: 'net-1', cursor: 'c0', limit: 10, vm_id: 'vm-1', connection_id: 'conn-1',
        event_type: 'request', decision: 'denied', since_unix_ms: 10, until_unix_ms: 20,
      }},
      {name: 'capsem_network_delete', arguments: {network_id: 'net-1'}},
    ];
    for (const call of calls) expect((await client.callTool(call)).isError, call.name).not.toBe(true);
    expect(requests.every(request => request.authorization === 'Bearer secret')).toBe(true);
    expect(requests.map(request => request.method)).toEqual([
      'POST', 'GET', 'GET', 'GET', 'PUT', 'GET', 'DELETE', 'GET', 'DELETE',
    ]);
    expect(JSON.parse(requests[0]?.body.toString() ?? '')).toEqual({name: 'team'});
    expect(requests[7]?.url).toContain('decision=denied');
    expect(requests[7]?.url).toContain('connection=conn-1');
    expect(requests[7]?.url).toContain('vm=vm-1');
  });
});
