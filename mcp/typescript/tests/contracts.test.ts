import {createServer as createHttpServer} from 'node:http';
import type {AddressInfo} from 'node:net';
import {HttpError, NetworkError} from '@capsem/sdk';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {InMemoryTransport} from '@modelcontextprotocol/sdk/inMemory.js';
import {afterEach, beforeEach, describe, expect, it} from 'vitest';
import {failure} from '../src/results.js';
import {createServer} from '../src/server.js';

describe('contracts', () => {
  let client: Client;
  let closeAll: () => Promise<void>;
  const requests: {method: string; url: string; body: Buffer}[] = [];

  beforeEach(async () => {
    requests.length = 0;
    const gateway = createHttpServer(async (request, response) => {
      const chunks: Buffer[] = [];
      for await (const chunk of request) chunks.push(Buffer.from(chunk));
      const record = {method: request.method ?? '', url: request.url ?? '', body: Buffer.concat(chunks)};
      requests.push(record);
      const path = new URL(record.url, 'http://gateway.test').pathname;
      const fixtures: Record<string, unknown> = {
        '/profiles/list': {profiles: []},
        '/profiles/code/mcp/info': {builtin_local_enabled: true, manual_server_count: 1, profile_id: 'code', server_count: 1},
        '/profiles/code/mcp/servers/list': [],
        '/profiles/code/mcp/default/info': {action: 'allow', source: 'profile'},
        '/profiles/code/mcp/servers/local/tools/list': [],
        '/profiles/code/mcp/servers/local/refresh': {instances: 1, server_id: 'local', success: true},
        '/profiles/code/mcp/servers/local/tools/read_file/call': {content: 'ok'},
      };
      if (!(path in fixtures)) return response.writeHead(404).end();
      response.setHeader('content-type', 'application/json');
      response.end(JSON.stringify(fixtures[path]));
    });
    await new Promise<void>(resolve => gateway.listen(0, '127.0.0.1', resolve));
    const port = (gateway.address() as AddressInfo).port;
    const server = createServer({gatewayUrl: `http://127.0.0.1:${port}`, token: 'secret', timeoutMs: 5_000});
    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    client = new Client({name: 'contracts-test', version: '1'});
    await server.connect(serverTransport);
    await client.connect(clientTransport);
    closeAll = async () => {
      await client.close();
      await server.close();
      await new Promise<void>((resolve, reject) => gateway.close(error => error ? reject(error) : resolve()));
    };
  });

  afterEach(async () => closeAll());

  it('uses canonical names and SDK-aligned parameter names', async () => {
    const tools = (await client.listTools()).tools;
    const names = new Set(tools.map(tool => tool.name));
    for (const name of ['capsem_status', 'capsem_pause', 'capsem_host_logs', 'capsem_mcp_call']) {
      expect(names.has(name), name).toBe(true);
    }
    for (const name of ['capsem_version', 'capsem_suspend', 'capsem_service_logs']) {
      expect(names.has(name), name).toBe(false);
    }
    const create = tools.find(tool => tool.name === 'capsem_create');
    expect(Object.keys(create?.inputSchema.properties ?? {})).toEqual(expect.arrayContaining([
      'profile', 'name', 'vcpu', 'memory', 'env', 'networks', 'container',
    ]));
    const exec = tools.find(tool => tool.name === 'capsem_exec');
    expect(Object.keys(exec?.inputSchema.properties ?? {})).toEqual(expect.arrayContaining(['vm_id', 'command', 'timeout_secs']));
    expect(tools.every(tool => Boolean(tool.description))).toBe(true);
  });

  it('exposes profile MCP discovery and invocation only through typed SDK calls', async () => {
    const calls = [
      {name: 'capsem_profiles', arguments: {}},
      {name: 'capsem_mcp_info', arguments: {}},
      {name: 'capsem_mcp_servers', arguments: {profile: 'code'}},
      {name: 'capsem_mcp_default', arguments: {profile: 'code'}},
      {name: 'capsem_mcp_tools', arguments: {profile: 'code', server_id: 'local'}},
      {name: 'capsem_mcp_refresh', arguments: {profile: 'code', server_id: 'local'}},
      {name: 'capsem_mcp_call', arguments: {
        profile: 'code', server_id: 'local', tool_id: 'read_file', arguments: {path: '/tmp/x'},
      }},
    ];
    for (const call of calls) expect((await client.callTool(call)).isError, call.name).not.toBe(true);
    expect(requests.map(request => request.method)).toEqual(['GET', 'GET', 'GET', 'GET', 'GET', 'POST', 'POST']);
    expect(JSON.parse(requests.at(-1)?.body.toString() ?? '')).toEqual({path: '/tmp/x'});
  });
});

describe('structured errors', () => {
  it.each([
    [new HttpError(403, 'secret response body'), 'http', 403],
    [new NetworkError(new Error('token-bearing low-level error')), 'network', undefined],
    [new TypeError('Invalid memory'), 'invalid_input', undefined],
    [new DOMException('caller secret', 'AbortError'), 'cancelled', undefined],
    [new DOMException('deadline detail', 'TimeoutError'), 'timeout', undefined],
    [new Error('internal secret'), 'internal', undefined],
  ])('returns machine-readable %s failures without raw causes', (error, kind, status) => {
    const result = failure(error);
    expect(result.isError).toBe(true);
    expect(result.structuredContent).toEqual({error: {kind, ...(status === undefined ? {} : {status})}});
    const encoded = JSON.stringify(result);
    expect(encoded).not.toContain('secret');
    expect(encoded).not.toContain('response body');
    expect(encoded).not.toContain('deadline detail');
  });
});
