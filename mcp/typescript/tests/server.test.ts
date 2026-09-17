import {spawnSync} from 'node:child_process';
import {mkdtempSync, rmSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';
import {InMemoryTransport} from '@modelcontextprotocol/sdk/inMemory.js';
import {describe, expect, it} from 'vitest';
import {parseConfig} from '../src/config.js';
import {createServer} from '../src/server.js';

describe('host MCP foundation', () => {
  it('takes the token from the environment or a file, never from argv', () => {
    const url = ['--gateway-url', 'http://127.0.0.1:19222'];
    expect(parseConfig([...url, '--timeout-ms', '45000'], {CAPSEM_GATEWAY_TOKEN: 'secret'}))
      .toEqual({gatewayUrl: 'http://127.0.0.1:19222', token: 'secret', timeoutMs: 45_000});
    const directory = mkdtempSync(join(tmpdir(), 'capsem-mcp-token-'));
    try {
      const file = join(directory, 'token');
      writeFileSync(file, 'from-file\n');
      expect(parseConfig([...url, '--token-file', file], {CAPSEM_GATEWAY_TOKEN: 'ignored'}).token).toBe('from-file');
      writeFileSync(file, '\n');
      expect(() => parseConfig([...url, '--token-file', file], {})).toThrow(/token-file is empty/);
      expect(() => parseConfig([...url, '--token-file', join(directory, 'missing')], {})).toThrow(/cannot read the token file/);
    } finally {
      rmSync(directory, {recursive: true, force: true});
    }
    for (const argv of [[], url]) {
      expect(() => parseConfig(argv, {})).toThrow(/gateway URL.*token/i);
    }
    expect(() => parseConfig(['--timeout-ms', '5'], {CAPSEM_GATEWAY_TOKEN: 'secret'})).toThrow(/gateway URL/i);
  });

  it.each([['--token', 'argv-secret'], ['--token=argv-secret', 'x']])(
    'refuses a token on the command line, where ps and /proc can read it (%s)',
    (...flag) => {
      let message = '';
      try {
        parseConfig(['--gateway-url', 'http://127.0.0.1:19222', ...flag], {CAPSEM_GATEWAY_TOKEN: 'env'});
      } catch (error) {
        message = error instanceof TypeError ? error.message : 'not a TypeError';
      }
      expect(message).toMatch(/CAPSEM_GATEWAY_TOKEN.*--token-file/);
      expect(message).not.toContain('argv-secret');
    },
  );

  it('reports why startup failed on stderr without echoing values', () => {
    const cli = fileURLToPath(new URL('../dist/cli.js', import.meta.url));
    const result = spawnSync(process.execPath, [cli, '--gateway-url', 'http://127.0.0.1:19222', '--token', 'argv-secret'], {
      encoding: 'utf8', env: {PATH: process.env.PATH ?? ''}, timeout: 10_000,
    });
    expect(result.status).toBe(1);
    expect(result.stdout).toBe('');
    expect(result.stderr).toMatch(/^capsem-mcp: .*--token-file/);
    expect(result.stderr).not.toContain('argv-secret');
  });

  it('returns structured MCP errors without exposing credentials', async () => {
    const server = createServer({gatewayUrl: 'http://127.0.0.1:19222', token: 'secret', timeoutMs: 30_000});
    const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
    const client = new Client({name: 'capsem-mcp-test', version: '1'});
    try {
      await server.connect(serverTransport);
      await client.connect(clientTransport);
      const result = await client.callTool({name: 'capsem_status', arguments: {}});
      expect(result.isError).toBe(true);
      expect(JSON.stringify(result)).not.toContain('secret');
    } finally {
      await client.close();
      await server.close();
    }
  });

  it('serves MCP over the packed executable stdout channel', async () => {
    const transport = new StdioClientTransport({
      command: process.execPath,
      args: [fileURLToPath(new URL('../dist/cli.js', import.meta.url)), '--gateway-url',
        'http://127.0.0.1:19222'],
      env: {PATH: process.env.PATH ?? '', CAPSEM_GATEWAY_TOKEN: 'secret'},
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
