import {createServer as createHttpServer, type IncomingMessage, type ServerResponse} from 'node:http';
import type {AddressInfo} from 'node:net';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {InMemoryTransport} from '@modelcontextprotocol/sdk/inMemory.js';
import {afterEach, beforeEach, describe, expect, it} from 'vitest';
import {createServer} from '../src/server.js';
import {codeProfile, customProfile, hypervisorInfo, provision, routeFixtures, sandbox} from './host-fixtures.js';

interface RequestRecord {method: string; url: string; authorization?: string; body: Buffer}

async function body(request: IncomingMessage): Promise<Buffer> {
  const chunks: Buffer[] = [];
  for await (const chunk of request) chunks.push(Buffer.from(chunk));
  return Buffer.concat(chunks);
}

function json(response: ServerResponse, value: object, status = 200): void {
  response.writeHead(status, {'content-type': 'application/json'});
  response.end(JSON.stringify(value));
}

function structured(result: unknown): Record<string, unknown> {
  const call = result as {isError?: boolean; structuredContent?: Record<string, unknown>};
  expect(call.isError).not.toBe(true);
  return call.structuredContent ?? {};
}

describe('host-tools', () => {
  const requests: RequestRecord[] = [];
  let gatewayUrl = '';
  let closeGateway: () => Promise<void>;
  let client: Client;
  let closeMcp: () => Promise<void>;

  beforeEach(async () => {
    requests.length = 0;
    const gateway = createHttpServer(async (request, response) => {
      const record: RequestRecord = {
        method: request.method ?? '', url: request.url ?? '', body: await body(request),
        ...(request.headers.authorization === undefined ? {} : {authorization: request.headers.authorization}),
      };
      requests.push(record);
      if (record.authorization !== 'Bearer gateway-secret') return json(response, {error: 'denied'}, 401);
      const path = new URL(record.url, 'http://gateway.test').pathname;
      if (path === '/status') return json(response, hypervisorInfo);
      if (path === '/vms/list') return json(response, {sandboxes: [sandbox]});
      if (path === '/profiles/list') return json(response, {profiles: [customProfile, codeProfile]});
      if (path === '/vms/create') return json(response, provision);
      if (path === '/networks/net-1') return json(response, {
        id: 'net-1', name: 'private', subnet: '10.0.0.0/24', created_unix_ms: 1, members: [],
      });
      if (path === '/vms/vm-1/info') return json(response, sandbox);
      if (path === '/vms/vm-1/exec' || path === '/run') {
        return json(response, {
          exit_code: 0,
          stderr: {encoding: 'utf8', data: ''},
          stdout: {encoding: 'utf8', data: 'ok'},
        });
      }
      if (path === '/vms/vm-1/files/content' && record.method === 'GET') {
        response.writeHead(200, {'content-type': 'application/octet-stream'});
        return response.end('hello');
      }
      if (path === '/vms/vm-1/files/content' && record.method === 'POST') {
        return json(response, {success: true, size: record.body.byteLength, vm_path: '/root/b.bin', container_path: '/workspace/b.bin'});
      }
      if (path === '/host-logs/service') return json(response, {source: 'service', text: 'ready'});
      if (path === '/host-logs/mcp') return json(response, {error: 'must-not-leak gateway-secret'}, 403);
      const fixture = routeFixtures[`${record.method} ${path}`];
      if (fixture) return json(response, fixture);
      return json(response, {error: `unexpected ${record.method} ${record.url}`}, 404);
    });
    await new Promise<void>(resolve => gateway.listen(0, '127.0.0.1', resolve));
    const address = gateway.address() as AddressInfo;
    gatewayUrl = `http://127.0.0.1:${address.port}`;
    closeGateway = () => new Promise((resolve, reject) => gateway.close(error => error ? reject(error) : resolve()));

    const server = createServer({gatewayUrl, token: 'gateway-secret', timeoutMs: 5_000});
    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    client = new Client({name: 'host-tools-test', version: '1'});
    await server.connect(serverTransport);
    await client.connect(clientTransport);
    closeMcp = async () => {
      await client.close();
      await server.close();
    };
  });

  afterEach(async () => {
    await closeMcp();
    await closeGateway();
  });

  it('publishes the normalized lifecycle and diagnostic surface', async () => {
    const names = (await client.listTools()).tools.map(tool => tool.name);
    expect(names).toContain('capsem_status');
    expect(names).toContain('capsem_pause');
    expect(names).toContain('capsem_host_logs');
    expect(names).toContain('capsem_stats_detail');
    expect(names).toContain('capsem_container_status');
    expect(names).toContain('capsem_port_open');
    expect(names).not.toContain('capsem_suspend');
    expect(names).not.toContain('capsem_version');
    expect(names).not.toContain('capsem_service_logs');
    expect(new Set(names).size).toBe(names.length);
  });

  it('uses typed SDK lifecycle calls with the supplied bearer credential', async () => {
    expect(structured(await client.callTool({name: 'capsem_list', arguments: {}}))).toEqual({sandboxes: [sandbox]});
    expect(structured(await client.callTool({name: 'capsem_info', arguments: {vm_id: 'vm-1'}}))).toEqual(sandbox);
    expect(structured(await client.callTool({
      name: 'capsem_exec', arguments: {vm_id: 'vm-1', command: 'printf ok', timeout_secs: 7},
    }))).toEqual({
      exit_code: 0,
      stderr: {encoding: 'utf8', data: ''},
      stdout: {encoding: 'utf8', data: 'ok'},
    });
    expect(requests.every(request => request.authorization === 'Bearer gateway-secret')).toBe(true);
    expect(requests.at(-1)?.method).toBe('POST');
    expect(JSON.parse(requests.at(-1)?.body.toString() ?? '')).toEqual({command: 'printf ok', timeout_secs: 7});
  });

  it('passes typed create resources and environment through the SDK without echoing secrets', async () => {
    const result = await client.callTool({
      name: 'capsem_create',
      arguments: {profile: 'co-work', name: 'demo', cpus: 2, memory: 2, env: {API_KEY: 'guest-secret'}, network_ids: ['net-1']},
    });
    expect(structured(result)).toEqual({id: 'vm-1', name: 'demo'});
    expect(JSON.stringify(result)).not.toContain('guest-secret');
    expect(JSON.parse(requests.at(-1)?.body.toString() ?? '')).toEqual({
      profile_id: 'co-work', name: 'demo', persistent: true, cpus: 2, ram_mb: 2048,
      env: {API_KEY: 'guest-secret'}, networks: ['private'],
    });
    expect(requests.some(request => request.url === '/profiles/list')).toBe(true);
  });

  it('creates and inspects containers and manages ports through SDK resources', async () => {
    const created = await client.callTool({
      name: 'capsem_create',
      arguments: {
        profile: 'code',
        env: {APP_SECRET: 'container-secret'},
        image: 'registry.example/app:latest', command: ['serve'],
        registry: {username: 'robot', password: 'registry-secret'},
      },
    });
    expect(structured(created)).toEqual({id: 'vm-1', name: 'demo'});
    expect(JSON.stringify(created)).not.toContain('container-secret');
    expect(JSON.stringify(created)).not.toContain('registry-secret');
    const body = JSON.parse(requests.at(-1)?.body.toString() ?? '');
    expect(body.env).toBeNull();
    expect(body.container).toMatchObject({
      image: 'registry.example/app:latest', args: ['serve'], env: {APP_SECRET: 'container-secret'},
      registry: {username: 'robot', password: 'registry-secret'}, attach: false,
    });

    expect(structured(await client.callTool({name: 'capsem_container_status', arguments: {vm_id: 'vm-1'}})))
      .toEqual({image: 'docker://busybox:latest', state: 'running'});
    expect(structured(await client.callTool({
      name: 'capsem_port_open', arguments: {vm_id: 'vm-1', guest_port: 8080},
    }))).toMatchObject({id: '49152', host: 49152, authenticate: false});
    expect(structured(await client.callTool({name: 'capsem_port_list', arguments: {vm_id: 'vm-1'}})))
      .toEqual({ports: [{id: '49152', guest: 8080, host: 49152, authenticate: false}]});
    const missingPort = await client.callTool({
      name: 'capsem_port_close', arguments: {vm_id: 'vm-1', port_id: 'missing'},
    });
    expect(missingPort.isError).toBe(true);
    expect(missingPort.structuredContent).toEqual({error: {kind: 'invalid_input'}});
    expect(structured(await client.callTool({
      name: 'capsem_port_close', arguments: {vm_id: 'vm-1', port_id: '49152'},
    }))).toEqual({success: true});
    expect(JSON.parse(requests.at(-5)?.body.toString() ?? '')).toEqual({
      target: 'container', access: 'loopback_tcp', guest_port: 8080, host_port: 0,
    });
    expect(requests.slice(-8).map(request => `${request.method} ${new URL(request.url, 'http://x').pathname}`)).toEqual([
      'POST /vms/create', 'GET /vms/vm-1/container', 'GET /vms/vm-1/container', 'POST /vms/vm-1/exposures',
      'GET /vms/vm-1/exposures', 'GET /vms/vm-1/exposures', 'GET /vms/vm-1/exposures',
      'DELETE /vms/vm-1/exposures/49152',
    ]);
  });

  it('rejects container options without an image', async () => {
    const result = await client.callTool({
      name: 'capsem_create',
      arguments: {command: ['true']},
    });
    expect(result.isError).toBe(true);
    expect(requests).toHaveLength(0);
  });

  // The agent opening an authenticated port is the one who hands the browser
  // its bootstrap token, so the tool result must carry it even though the SDK
  // keeps it out of the Port's enumerable (logged) fields.
  it('returns the bootstrap token for an authenticated port', async () => {
    const opened = structured(await client.callTool({
      name: 'capsem_port_open', arguments: {vm_id: 'vm-1', guest_port: 8080, authenticate: true},
    }));
    expect(opened).toMatchObject({
      id: '49152', host: null,
      url: 'http://49152.localhost:19223/_capsem/bootstrap', bootstrapToken: 'bootstrap-secret',
    });
  });

  // A name the catalog does not have is the caller's mistake, not ours.
  // No profile means the catalog default the gateway names, not a literal.
  it('creates with the catalog default when no profile is named', async () => {
    const created = await client.callTool({name: 'capsem_create', arguments: {}});
    expect(created.isError).not.toBe(true);
    const body = requests.find(request => request.url === '/vms/create')?.body.toString() ?? '';
    expect(JSON.parse(body)).toMatchObject({profile_id: 'code'});
    expect(requests.some(request => request.url === '/status')).toBe(true);
  });

  it('reports an unknown profile as invalid input', async () => {
    const result = await client.callTool({
      name: 'capsem_create', arguments: {profile: 'ghost'},
    });
    expect(result.isError).toBe(true);
    expect(result.structuredContent).toEqual({error: {kind: 'invalid_input'}});
    expect(JSON.stringify(result)).toContain('ghost');
  });

  it('bounds a file read and reports what it returned', async () => {
    const whole = structured(await client.callTool({
      name: 'capsem_read_file', arguments: {vm_id: 'vm-1', path: '/workspace/a.txt'},
    }));
    expect(whole).toEqual({path: '/workspace/a.txt', encoding: 'utf8', size: 5, offset: 0, content: 'hello', truncated: false});
    const bounded = structured(await client.callTool({
      name: 'capsem_read_file', arguments: {vm_id: 'vm-1', path: '/workspace/a.txt', offset: 1, max_bytes: 2},
    }));
    expect(bounded).toEqual({path: '/workspace/a.txt', encoding: 'utf8', size: 5, offset: 1, content: 'el', truncated: true});
    const past = structured(await client.callTool({
      name: 'capsem_read_file', arguments: {vm_id: 'vm-1', path: '/workspace/a.txt', offset: 99},
    }));
    expect(past).toMatchObject({content: '', offset: 99, size: 5, truncated: false});
  });

  it('transfers text and binary file content through the SDK byte APIs', async () => {
    const read = await client.callTool({
      name: 'capsem_read_file', arguments: {vm_id: 'vm-1', path: '/workspace/a.txt'},
    });
    expect(structured(read)).toMatchObject({path: '/workspace/a.txt', encoding: 'utf8', size: 5, content: 'hello'});
    const write = await client.callTool({
      name: 'capsem_write_file',
      arguments: {vm_id: 'vm-1', path: '/workspace/b.bin', encoding: 'base64', content: 'AAEC'},
    });
    expect(structured(write)).toEqual({success: true, size: 3, vm_path: '/root/b.bin', container_path: '/workspace/b.bin'});
    expect(requests.at(-1)?.body).toEqual(Buffer.from([0, 1, 2]));
  });

  it('uses the consolidated host log route and redacts gateway error bodies', async () => {
    expect(structured(await client.callTool({name: 'capsem_host_logs', arguments: {}})))
      .toEqual({source: 'service', text: 'ready'});
    const denied = await client.callTool({name: 'capsem_host_logs', arguments: {source: 'mcp'}});
    expect(denied.isError).toBe(true);
    expect(JSON.stringify(denied)).toContain('HTTP 403');
    expect(JSON.stringify(denied)).not.toContain('gateway-secret');
    expect(requests.at(-2)?.url).toBe('/host-logs/service');
  });

  it('routes the remaining lifecycle and diagnostic tools through typed SDK resources', async () => {
    const calls: {name: string; arguments: Record<string, unknown>}[] = [
      {name: 'capsem_run', arguments: {command: 'true', profile: 'code', timeout_secs: 3}},
      {name: 'capsem_start', arguments: {vm_id: 'vm-1'}},
      {name: 'capsem_stop', arguments: {vm_id: 'vm-1'}},
      {name: 'capsem_pause', arguments: {vm_id: 'vm-1'}},
      {name: 'capsem_resume', arguments: {vm_id: 'vm-1'}},
      {name: 'capsem_delete', arguments: {vm_id: 'vm-1'}},
      {name: 'capsem_fork', arguments: {vm_id: 'vm-1', name: 'copy', description: 'debug'}},
      {name: 'capsem_persist', arguments: {vm_id: 'vm-1', name: 'saved'}},
      {name: 'capsem_purge', arguments: {all: true}},
      {name: 'capsem_list_files', arguments: {vm_id: 'vm-1', path: '/workspace', depth: 2}},
      {name: 'capsem_vm_logs', arguments: {vm_id: 'vm-1', grep: 'boot', tail: 3}},
      {name: 'capsem_timeline', arguments: {vm_id: 'vm-1', layers: ['exec', 'net'], limit: 5}},
      {name: 'capsem_history', arguments: {vm_id: 'vm-1', limit: 5, offset: 0, search: 'cargo'}},
      {name: 'capsem_stats', arguments: {vm_id: 'vm-1'}},
      {name: 'capsem_stats_detail', arguments: {vm_id: 'vm-1'}},
      {name: 'capsem_snapshots', arguments: {vm_id: 'vm-1'}},
      {name: 'capsem_snapshot_status', arguments: {vm_id: 'vm-1'}},
      {name: 'capsem_file_history', arguments: {vm_id: 'vm-1', checkpoint: 'cp-1', limit: 10}},
      {name: 'capsem_panics', arguments: {since: '1h', limit: 4}},
      {name: 'capsem_triage', arguments: {vm_id: 'vm-1', since: '5m', limit: 2}},
    ];
    for (const call of calls) {
      const result = await client.callTool(call);
      expect(result.isError, `${call.name}: ${JSON.stringify(result.content)}`).not.toBe(true);
    }
    expect(requests.some(request => request.url.includes('layers=exec%2Cnet'))).toBe(true);
    expect(requests.some(request => request.url === '/panics?since=1h&limit=4')).toBe(true);
    expect(requests.some(request => request.url === '/triage?since=5m&limit=2&id=vm-1')).toBe(true);
    expect(JSON.parse(requests.find(request => request.url === '/purge')?.body.toString() ?? '')).toEqual({all: true});
  });
});
