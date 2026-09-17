import {createServer as createHttpServer, type IncomingMessage, type ServerResponse} from 'node:http';
import type {AddressInfo} from 'node:net';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {InMemoryTransport} from '@modelcontextprotocol/sdk/inMemory.js';
import {afterEach, beforeEach, describe, expect, it} from 'vitest';
import {createServer} from '../src/server.js';

interface RequestRecord {method: string; url: string; authorization?: string; body: Buffer}

const sandbox = {
  available_actions: ['pause', 'stop', 'fork', 'delete'], id: 'vm-1', name: 'demo', pid: 42,
  profile_id: 'code', status: 'Running',
};
const provision = {
  available_actions: ['pause', 'stop', 'fork', 'delete'], id: 'vm-1', name: 'demo',
  profile_id: 'code', status: 'Running',
};
const customProfile = {
  availability: {web: true, shell: true, mobile: false},
  default_rule_count: 0, description: 'Custom profile', id: 'co-work', mcp_server_count: 0,
  name: 'Co-work', plugin_count: 0, rule_count: 0, source: 'builtin',
  update_semantics: {
    new_sessions: 'use_current_profile_catalog', existing_vms: 'pinned_until_recreate',
    upgrade_action: 'recreate_vm',
  },
};

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
      if (path === '/vms/list') return json(response, {sandboxes: [sandbox]});
      if (path === '/profiles/list') return json(response, {profiles: [customProfile]});
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
        return json(response, {success: true, size: record.body.byteLength});
      }
      if (path === '/host-logs/service') return json(response, {source: 'service', text: 'ready'});
      if (path === '/panics') return json(response, {error: 'must-not-leak gateway-secret'}, 403);
      const fixtures: Record<string, object> = {
        'POST /vms/vm-1/start': provision,
        'POST /vms/vm-1/stop': {persistent: false, success: true},
        'POST /vms/vm-1/pause': {success: true},
        'POST /vms/vm-1/resume': provision,
        'DELETE /vms/vm-1/delete': {success: true},
        'POST /vms/vm-1/fork': {id: 'fork-1', name: 'copy', size_bytes: 12},
        'POST /vms/vm-1/save': {name: 'saved', success: true},
        'POST /purge': {ephemeral_purged: 1, persistent_purged: 0, purged: 1},
        'GET /vms/vm-1/files/list': {entries: []},
        'GET /vms/vm-1/logs': {logs: 'booted'},
        'GET /triage': {host: {errors: [], panics: [], slow_ops: []}, rank: [], session: {}, since: '5m'},
        'GET /vms/vm-1/timeline': {events: []},
        'GET /vms/vm-1/history': {commands: [], has_more: false, total: 0},
        'GET /vms/vm-1/stats/summary': {
          allowed_requests: 1, denied_requests: 0, total_estimated_cost: 0,
          total_input_tokens: 0, total_output_tokens: 0, total_requests: 1,
          total_thinking_tokens: 0, total_tool_calls: 0,
        },
        'GET /vms/vm-1/stats/detail': {
          audit_events: [], body_blobs: {}, credential_events: [], dns_events: [], file_events: [],
          http_events: [], interactions: {bodies: [], items: []}, model_events: [], model_stats: [],
          process_events: [], tool_events: [],
        },
        'GET /vms/vm-1/snapshots/list': {snapshots: [], total: 0},
        'GET /vms/vm-1/snapshots/status': {
          auto_count: 0, manual_available: 0, manual_count: 0, snapshots: [], total: 0,
        },
        'GET /vms/vm-1/changes': {changes: [], checkpoint: 'cp-1', has_more: false, total: 0},
        'GET /vms/vm-1/container': {image: 'docker://busybox:latest', state: 'running'},
        'POST /vms/vm-1/exposures': {
          access: 'loopback_tcp', id: '49152', host_port: 49152, guest_port: 8080, target: 'container',
        },
        'GET /vms/vm-1/exposures': {
          owner_generation: '7',
          exposures: [{
            access: 'loopback_tcp', id: '49152', host_port: 49152, guest_port: 8080, target: 'container',
          }],
        },
        'DELETE /vms/vm-1/exposures/49152': {success: true},
      };
      const fixture = fixtures[`${record.method} ${path}`];
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

  it('transfers text and binary file content through the SDK byte APIs', async () => {
    const read = await client.callTool({
      name: 'capsem_read_file', arguments: {vm_id: 'vm-1', path: '/workspace/a.txt'},
    });
    expect(structured(read)).toEqual({path: '/workspace/a.txt', encoding: 'utf8', size: 5, content: 'hello'});
    const write = await client.callTool({
      name: 'capsem_write_file',
      arguments: {vm_id: 'vm-1', path: '/workspace/b.bin', encoding: 'base64', content: 'AAEC'},
    });
    expect(structured(write)).toEqual({success: true, size: 3});
    expect(requests.at(-1)?.body).toEqual(Buffer.from([0, 1, 2]));
  });

  it('uses the consolidated host log route and redacts gateway error bodies', async () => {
    expect(structured(await client.callTool({name: 'capsem_host_logs', arguments: {}})))
      .toEqual({source: 'service', text: 'ready'});
    const denied = await client.callTool({name: 'capsem_panics', arguments: {}});
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
      {name: 'capsem_triage', arguments: {vm_id: 'vm-1', since: '5m', limit: 2}},
      {name: 'capsem_timeline', arguments: {vm_id: 'vm-1', layers: ['exec', 'net'], limit: 5}},
      {name: 'capsem_history', arguments: {vm_id: 'vm-1', limit: 5, offset: 0, search: 'cargo'}},
      {name: 'capsem_stats', arguments: {vm_id: 'vm-1'}},
      {name: 'capsem_stats_detail', arguments: {vm_id: 'vm-1'}},
      {name: 'capsem_snapshots', arguments: {vm_id: 'vm-1'}},
      {name: 'capsem_snapshot_status', arguments: {vm_id: 'vm-1'}},
      {name: 'capsem_file_history', arguments: {vm_id: 'vm-1', checkpoint: 'cp-1', limit: 10}},
    ];
    for (const call of calls) {
      expect((await client.callTool(call)).isError, call.name).not.toBe(true);
    }
    expect(requests.some(request => request.url.includes('layers=exec%2Cnet'))).toBe(true);
    expect(JSON.parse(requests.find(request => request.url === '/purge')?.body.toString() ?? '')).toEqual({all: true});
  });
});
