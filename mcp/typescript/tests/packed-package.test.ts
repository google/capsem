import {execFileSync} from 'node:child_process';
import {mkdtempSync, readFileSync, readdirSync, rmSync, symlinkSync} from 'node:fs';
import {createServer as createHttpServer} from 'node:http';
import type {AddressInfo} from 'node:net';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';
import {afterEach, describe, expect, it} from 'vitest';

interface PackedClient {client: Client; transport: StdioClientTransport; stderr: string[]}

describe('packed-package', () => {
  const fixtures: string[] = [];
  const clients: PackedClient[] = [];
  const gateways: ReturnType<typeof createHttpServer>[] = [];

  afterEach(async () => {
    for (const packed of clients.splice(0)) await packed.client.close();
    for (const gateway of gateways.splice(0)) {
      gateway.closeAllConnections();
      await new Promise<void>((resolve, reject) => gateway.close(error => error ? reject(error) : resolve()));
    }
    for (const fixture of fixtures.splice(0)) rmSync(fixture, {recursive: true, force: true});
  });

  function pack(): {cli: string; manifest: Record<string, unknown>} {
    const packageRoot = fileURLToPath(new URL('..', import.meta.url));
    const fixture = mkdtempSync(join(tmpdir(), 'capsem-mcp-pack-'));
    fixtures.push(fixture);
    execFileSync('pnpm', ['pack', '--config.ignore-scripts=true', '--pack-destination', fixture], {
      cwd: packageRoot,
      stdio: 'pipe',
    });
    const archive = readdirSync(fixture).find(name => name.endsWith('.tgz'));
    if (!archive) throw new Error('pnpm pack did not create a tarball');
    execFileSync('tar', ['-xzf', join(fixture, archive), '-C', fixture]);
    const extracted = join(fixture, 'package');
    symlinkSync(join(packageRoot, 'node_modules'), join(extracted, 'node_modules'), 'dir');
    return {
      cli: join(extracted, 'dist', 'cli.js'),
      manifest: JSON.parse(readFileSync(join(extracted, 'package.json'), 'utf8')) as Record<string, unknown>,
    };
  }

  async function connect(cli: string, gatewayUrl: string, token: string): Promise<PackedClient> {
    const transport = new StdioClientTransport({
      command: process.execPath,
      args: [cli, '--gateway-url', gatewayUrl, '--token', token, '--timeout-ms', '5000'],
      stderr: 'pipe',
    });
    const stderr: string[] = [];
    transport.stderr?.on('data', chunk => stderr.push(String(chunk)));
    const client = new Client({name: 'packed-package-test', version: '1'});
    await client.connect(transport);
    const packed = {client, transport, stderr};
    clients.push(packed);
    return packed;
  }

  it('drives isolated authenticated workflows through the tarball executable', async () => {
    const {cli, manifest} = pack();
    expect(manifest.bin).toEqual({'capsem-mcp': './dist/cli.js'});
    expect(manifest.dependencies).toMatchObject({'@capsem/sdk': '0.6.3'});

    const authorizations: string[] = [];
    let slowRequestClosed: (() => void) | undefined;
    const slowClosed = new Promise<void>(resolve => {slowRequestClosed = resolve;});
    let slowRequestStarted: (() => void) | undefined;
    const slowStarted = new Promise<void>(resolve => {slowRequestStarted = resolve;});
    let listRequestClosed: (() => void) | undefined;
    const listClosed = new Promise<void>(resolve => {listRequestClosed = resolve;});
    let listRequestStarted: (() => void) | undefined;
    const listStarted = new Promise<void>(resolve => {listRequestStarted = resolve;});
    const gateway = createHttpServer((request, response) => {
      authorizations.push(request.headers.authorization ?? '');
      const path = new URL(request.url ?? '/', 'http://gateway.test').pathname;
      if (path === '/networks') {
        response.setHeader('content-type', 'application/json');
        return response.end(JSON.stringify({networks: []}));
      }
      if (path === '/panics') return response.writeHead(403).end('private-denial-detail');
      if (path === '/status') {
        slowRequestStarted?.();
        request.on('close', () => slowRequestClosed?.());
        response.on('close', () => slowRequestClosed?.());
        return;
      }
      if (path === '/vms/list') {
        listRequestStarted?.();
        request.on('close', () => listRequestClosed?.());
        response.on('close', () => listRequestClosed?.());
        return;
      }
      response.writeHead(404).end();
    });
    gateways.push(gateway);
    await new Promise<void>(resolve => gateway.listen(0, '127.0.0.1', resolve));
    const gatewayUrl = `http://127.0.0.1:${(gateway.address() as AddressInfo).port}`;
    const [first, second] = await Promise.all([
      connect(cli, gatewayUrl, 'token-a'), connect(cli, gatewayUrl, 'token-b'),
    ]);

    const tools = await first.client.listTools();
    expect(tools.tools.some(tool => tool.name === 'capsem_network_list')).toBe(true);
    const [one, two] = await Promise.all([
      first.client.callTool({name: 'capsem_network_list', arguments: {}}),
      second.client.callTool({name: 'capsem_network_list', arguments: {}}),
    ]);
    expect(one.structuredContent).toEqual({networks: []});
    expect(two.structuredContent).toEqual({networks: []});
    expect(new Set(authorizations.slice(0, 2))).toEqual(new Set(['Bearer token-a', 'Bearer token-b']));

    const denied = await first.client.callTool({name: 'capsem_panics', arguments: {}});
    expect(denied.isError).toBe(true);
    expect(denied.structuredContent).toEqual({error: {kind: 'http', status: 403}});
    expect(JSON.stringify(denied)).not.toContain('private-denial-detail');

    const controller = new AbortController();
    const pending = first.client.callTool(
      {name: 'capsem_status', arguments: {}}, undefined, {signal: controller.signal},
    );
    await slowStarted;
    controller.abort();
    await expect(pending).rejects.toThrow(/AbortError/);
    await slowClosed;

    const listController = new AbortController();
    const pendingList = first.client.callTool(
      {name: 'capsem_list', arguments: {}}, undefined, {signal: listController.signal},
    );
    await listStarted;
    listController.abort();
    await expect(pendingList).rejects.toThrow(/AbortError/);
    await Promise.race([
      listClosed,
      new Promise<never>((_resolve, reject) => setTimeout(() => reject(new Error('cancelled tool kept its gateway request open')), 500)),
    ]);
    expect(first.stderr.join('')).toBe('');
    expect(second.stderr.join('')).toBe('');
  }, 15_000);
});
