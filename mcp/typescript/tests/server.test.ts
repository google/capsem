import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';
import {InMemoryTransport} from '@modelcontextprotocol/sdk/inMemory.js';
import {describe, expect, it} from 'vitest';
import {parseConfig, type Config} from '../src/config.js';
import {createServer} from '../src/server.js';
import {accessId, capability, delegated} from './delegated-fixture.js';

const url = ['--gateway-url', 'http://127.0.0.1:19222', '--access-id', accessId];

describe('host MCP foundation', () => {
  it('requires explicit delegated configuration and a bounded transport timeout', () => {
    expect(parseConfig([...url, '--timeout-ms', '45000'], {CAPSEM_AGENT_CAPABILITY: capability}))
      .toEqual({gatewayUrl: 'http://127.0.0.1:19222', ...delegated, timeoutMs: 45_000});
    for (const argv of [[], url, ['--timeout-ms', '5']]) {
      expect(() => parseConfig(argv, {})).toThrow(/gateway URL.*delegated.*capability/i);
    }
    for (const timeout of ['0', '-1', 'NaN', '9007199254740992']) {
      expect(() => parseConfig([...url, '--timeout-ms', timeout], {CAPSEM_AGENT_CAPABILITY: capability})).toThrow(/timeout/);
    }
    expect(() => parseConfig([...url, '--access-id', accessId], {CAPSEM_AGENT_CAPABILITY: capability})).toThrow(/duplicate/);
  });

  it.each([['--token', 'argv-secret'], ['--token=argv-secret', 'x']])(
    'refuses secret argv values without echo (%s)', (...flag) => {
      let message = '';
      try {parseConfig([...url, ...flag], {CAPSEM_AGENT_CAPABILITY: capability});} catch (error) {
        message = error instanceof TypeError ? error.message : 'not a TypeError';
      }
      expect(message).toMatch(/CAPSEM_AGENT_CAPABILITY/);
      expect(message).not.toContain('argv-secret');
    },
  );

  it('reports why startup failed on stderr without echoing values', () => {
    const cli = fileURLToPath(new URL('../dist/cli.js', import.meta.url));
    const result = spawnSync(process.execPath, [cli, ...url, '--token', 'argv-secret'], {
      encoding: 'utf8', env: {PATH: process.env.PATH ?? ''}, timeout: 10_000,
    });
    expect(result.status).toBe(1);
    expect(result.stdout).toBe('');
    expect(result.stderr).toMatch(/^capsem-mcp: .*CAPSEM_AGENT_CAPABILITY/);
    expect(result.stderr).not.toContain('argv-secret');
  });

  it('never lets programmatic callers select operator discovery or a naked operator token', async () => {
    const old = {gatewayUrl: 'http://127.0.0.1:19222', token: 'operator-secret', timeoutMs: 30_000};
    expect(() => createServer(old as unknown as Config)).toThrow(/delegated.*capability/i);
    expect(() => createServer({...delegated, gatewayUrl: undefined as unknown as string, timeoutMs: 30_000})).toThrow(/explicit gateway URL/);
    const config = {...delegated, gatewayUrl: 'http://127.0.0.1:19222', timeoutMs: 30_000};
    Object.defineProperty(config, 'token', {get() {throw new Error('must not read operator token');}});
    await createServer(config).close();
  });

  it('returns structured MCP errors without exposing credentials', async () => {
    const server = createServer({gatewayUrl: 'http://127.0.0.1:19222', ...delegated, timeoutMs: 30_000});
    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    const client = new Client({name: 'capsem-mcp-test', version: '1'});
    try {
      await server.connect(serverTransport);
      await client.connect(clientTransport);
      const result = await client.callTool({name: 'capsem_status', arguments: {}});
      expect(result.isError).toBe(true);
      expect(JSON.stringify(result)).not.toContain(capability);
    } finally {
      await client.close();
      await server.close();
    }
  });

  it('serves MCP over the built executable stdout channel with delegated input', async () => {
    const transport = new StdioClientTransport({
      command: process.execPath,
      args: [fileURLToPath(new URL('../dist/cli.js', import.meta.url)), ...url],
      env: {PATH: process.env.PATH ?? '', CAPSEM_AGENT_CAPABILITY: capability},
      stderr: 'pipe',
    });
    const client = new Client({name: 'capsem-mcp-test', version: '1'});
    try {
      await client.connect(transport);
      const names = (await client.listTools()).tools.map(tool => tool.name);
      expect(names).toContain('capsem_status');
      expect(names).toContain('capsem_exec');
    } finally {
      await client.close();
    }
  });
});
