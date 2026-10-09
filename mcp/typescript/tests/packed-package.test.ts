import {execFileSync} from 'node:child_process';
import {copyFileSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, writeFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {createServer as createHttpServer} from 'node:http';
import type {AddressInfo} from 'node:net';
import {tmpdir} from 'node:os';
import {dirname, join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';
import {afterEach, describe, expect, it} from 'vitest';

interface PackedClient {client: Client; transport: StdioClientTransport; stderr: string[]}

describe('packed-package', () => {
  const fixtures: string[] = [];
  const clients: PackedClient[] = [];
  const gateways: ReturnType<typeof createHttpServer>[] = [];
  const retained: {path: string; sha256: string}[] = [];

  afterEach(async () => {
    for (const packed of clients.splice(0)) await packed.client.close();
    for (const gateway of gateways.splice(0)) {
      gateway.closeAllConnections();
      await new Promise<void>((resolve, reject) => gateway.close(error => error ? reject(error) : resolve()));
    }
    for (const fixture of fixtures.splice(0)) rmSync(fixture, {recursive: true, force: true});
    for (const archive of retained.splice(0)) expect(digest(archive.path)).toBe(archive.sha256);
  });

  const digest = (path: string): string => createHash('sha256').update(readFileSync(path)).digest('hex');

  function pack(): {cli: string; dependencies: string; manifest: Record<string, unknown>} {
    const packageRoot = fileURLToPath(new URL('..', import.meta.url));
    const fixture = mkdtempSync(join(tmpdir(), 'capsem-mcp-pack-'));
    fixtures.push(fixture);
    const sources = {sdk: join(packageRoot, '../../sdk/typescript'), mcp: packageRoot};
    const archives: string[] = [];
    for (const [owner, source] of Object.entries(sources)) {
      const destination = join(fixture, owner);
      mkdirSync(destination);
      const selectedDirectory = process.env.CAPSEM_MCP_PACKAGE_ARCHIVES;
      if (selectedDirectory) {
        const version = (JSON.parse(readFileSync(join(source, 'package.json'), 'utf8')) as {version: string}).version;
        const path = join(selectedDirectory, `capsem-${owner}-${version}.tgz`);
        retained.push({path, sha256: digest(path)});
        copyFileSync(path, join(destination, `capsem-${owner}-${version}.tgz`));
      } else execFileSync('pnpm', ['pack', '--config.ignore-scripts=true', '--pack-destination', destination], {
        cwd: source, stdio: 'pipe', timeout: 15_000,
        env: {...process.env, HOME: fixture},
      });
      const name = readdirSync(destination).find(entry => entry.endsWith('.tgz'));
      if (!name) throw new Error('pnpm pack did not create a tarball');
      const archive = join(destination, name);
      if (selectedDirectory) expect(digest(archive)).toBe(retained.at(-1)?.sha256);
      archives.push(archive);
      execFileSync('tar', ['-xzf', archive, '-C', destination], {timeout: 5000});
    }
    writeFileSync(join(fixture, 'package.json'), JSON.stringify({name: 'capsem-mcp-clean-consumer', private: true}));
    const npmCache = process.env.NPM_CONFIG_CACHE
      ?? (process.env.HOME && existsSync(process.env.HOME) ? join(process.env.HOME, '.npm') : undefined);
    execFileSync('npm', ['install', '--offline', '--omit=dev', '--ignore-scripts', '--no-audit', '--no-fund', ...archives], {
      cwd: fixture, stdio: 'pipe', timeout: 60_000,
      env: {HOME: fixture, PATH: process.env.PATH ?? '',
        ...(npmCache ? {NPM_CONFIG_CACHE: npmCache} : {})},
    });
    const dependencies = join(fixture, 'node_modules');
    for (const [owner, source] of Object.entries(sources)) {
      const installed = join(dependencies, '@capsem', owner);
      expect(lstatSync(installed).isSymbolicLink()).toBe(false);
      comparePayload(join(fixture, owner, 'package'), installed);
      comparePayload(join(source, 'dist'), join(installed, 'dist'));
    }
    const installed = join(dependencies, '@capsem/mcp');
    return {
      cli: join(installed, 'dist', 'cli.js'), dependencies,
      manifest: JSON.parse(readFileSync(join(installed, 'package.json'), 'utf8')) as Record<string, unknown>,
    };
  }

  function comparePayload(expected: string, actual: string): void {
    expect(lstatSync(actual).isSymbolicLink()).toBe(false);
    const names = readdirSync(expected).sort();
    expect(readdirSync(actual).sort()).toEqual(names);
    for (const name of names) {
      const from = join(expected, name), to = join(actual, name);
      expect(lstatSync(to).isSymbolicLink()).toBe(false);
      if (lstatSync(from).isDirectory()) comparePayload(from, to);
      else expect(readFileSync(to)).toEqual(readFileSync(from));
    }
  }

  async function connect(cli: string, gatewayUrl: string, token: string): Promise<PackedClient> {
    const consumerNode = process.env.CAPSEM_MCP_PACKAGE_NODE ?? process.execPath;
    const runtimeVersion = execFileSync(consumerNode, ['-p', 'process.version'], {encoding: 'utf8', timeout: 5000}).trim();
    expect(Number(runtimeVersion.slice(1).split('.')[0])).toBeGreaterThanOrEqual(20);
    const transport = new StdioClientTransport({
      command: consumerNode,
      args: [cli, '--gateway-url', gatewayUrl, '--timeout-ms', '5000'],
      env: {HOME: dirname(cli), PATH: process.env.PATH ?? '', CAPSEM_GATEWAY_TOKEN: token},
      stderr: 'pipe',
    });
    const stderr: string[] = [];
    transport.stderr?.on('data', chunk => stderr.push(String(chunk)));
    const client = new Client({name: 'packed-package-test', version: '1'});
    await client.connect(transport);
    expect(transport.pid).toBeGreaterThan(0);
    if (process.platform === 'linux') {
      expect(realpathSync(`/proc/${transport.pid}/exe`)).toBe(realpathSync(consumerNode));
    }
    const packed = {client, transport, stderr};
    clients.push(packed);
    return packed;
  }

  it('drives isolated authenticated workflows through the tarball executable', async () => {
    const {cli, dependencies, manifest} = pack();
    expect(lstatSync(dependencies).isSymbolicLink()).toBe(false);
    expect(manifest.bin).toEqual({'capsem-mcp': './dist/cli.js'});
    expect(manifest.dependencies).toMatchObject({'@capsem/sdk': manifest.version});

    const authorizations: string[] = [];
    const pin = `registry.example/code@sha256:${'a'.repeat(64)}`;
    const imageRequests: {path: string; method: string | undefined; body: string}[] = [];
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
      if (path === '/images' || path === '/images/pull') {
        let body = '';
        request.setEncoding('utf8');
        request.on('data', (chunk: string) => {body += chunk;});
        request.on('end', () => {
          imageRequests.push({path: request.url ?? '', method: request.method, body});
          response.setHeader('content-type', 'application/json');
          response.end(JSON.stringify(path === '/images'
            ? {images: [{name: 'code', description: 'Tools', architectures: ['amd64'], image: pin, cached: 'unknown'}]}
            : {image: 'code', resolved: pin, digest: `sha256:${'b'.repeat(64)}`}));
        });
        return;
      }
      if (path === '/networks') {
        response.setHeader('content-type', 'application/json');
        return response.end(JSON.stringify({networks: []}));
      }
      if (path === '/host-logs/mcp') return response.writeHead(403).end('private-denial-detail');
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
    for (const connection of [first, second]) {
      expect(connection.client.getServerVersion()).toEqual({name: 'capsem-mcp', version: manifest.version});
    }

    const tools = await first.client.listTools();
    expect(tools.tools.some(tool => tool.name === 'capsem_network_list')).toBe(true);
    const [one, two] = await Promise.all([
      first.client.callTool({name: 'capsem_network_list', arguments: {}}),
      second.client.callTool({name: 'capsem_network_list', arguments: {}}),
    ]);
    expect(one.structuredContent).toEqual({networks: []});
    expect(two.structuredContent).toEqual({networks: []});
    expect(new Set(authorizations.slice(0, 2))).toEqual(new Set(['Bearer token-a', 'Bearer token-b']));

    expect(tools.tools.map(tool => tool.name)).toEqual(expect.arrayContaining(['capsem_image_list', 'capsem_image_pull']));
    const catalog = await first.client.callTool({name: 'capsem_image_list', arguments: {refresh: true}});
    expect(catalog.isError).not.toBe(true);
    expect(catalog.structuredContent).toEqual({images: [{name: 'code', description: 'Tools', architectures: ['amd64'], image: pin, cached: 'unknown'}]});
    const pulled = await first.client.callTool({name: 'capsem_image_pull', arguments: {image: 'code'}});
    expect(pulled.isError).not.toBe(true);
    expect(pulled.structuredContent).toEqual({image: 'code', resolved: pin, digest: `sha256:${'b'.repeat(64)}`});
    expect(imageRequests).toEqual([
      {method: 'GET', path: '/images?refresh=true', body: ''},
      {method: 'POST', path: '/images/pull', body: '{"image":"code"}'},
    ]);
    expect(authorizations.slice(2, 4)).toEqual(['Bearer token-a', 'Bearer token-a']);

    const denied = await first.client.callTool({name: 'capsem_host_logs', arguments: {source: 'mcp'}});
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
    if (process.env.CAPSEM_MCP_PACKAGE_RECEIPT) {
      writeFileSync(process.env.CAPSEM_MCP_PACKAGE_RECEIPT, JSON.stringify({archives: retained,
        version: manifest.version, node: execFileSync(process.env.CAPSEM_MCP_PACKAGE_NODE ?? process.execPath,
          ['-p', 'process.version'], {encoding: 'utf8', timeout: 5000}).trim(), ok: true}, null, 2) + '\n');
    }
  }, 90_000);
});
