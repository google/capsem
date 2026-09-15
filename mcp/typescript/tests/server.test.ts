import {fileURLToPath} from 'node:url';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {StdioClientTransport} from '@modelcontextprotocol/sdk/client/stdio.js';
import {InMemoryTransport} from '@modelcontextprotocol/sdk/inMemory.js';
import {describe, expect, it} from 'vitest';
import {parseConfig} from '../src/config.js';
import {createServer} from '../src/server.js';

describe('host MCP foundation', () => {
  it('requires explicit gateway credentials and keeps token values out of errors', () => {
    expect(parseConfig(['--gateway-url', 'http://127.0.0.1:19222', '--token', 'secret', '--timeout-ms', '45000']))
      .toEqual({gatewayUrl: 'http://127.0.0.1:19222', token: 'secret', timeoutMs: 45_000});
    for (const argv of [[], ['--gateway-url', 'http://127.0.0.1:19222'], ['--token', 'secret']]) {
      expect(() => parseConfig(argv)).toThrow(/gateway URL.*token/i);
    }
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
        'http://127.0.0.1:19222', '--token', 'secret'],
      stderr: 'pipe',
    });
    const client = new Client({name: 'capsem-mcp-test', version: '1'});
    try {
      await client.connect(transport);
      expect((await client.listTools()).tools.map(tool => tool.name)).toEqual(['capsem_status']);
    } finally {
      await client.close();
    }
  });
});
