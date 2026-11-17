import {HttpError} from '@capsem/sdk';
import {Client} from '@modelcontextprotocol/sdk/client/index.js';
import {InMemoryTransport} from '@modelcontextprotocol/sdk/inMemory.js';
import {McpServer} from '@modelcontextprotocol/sdk/server/mcp.js';
import {afterEach, expect, it, vi} from 'vitest';
import {DIAGNOSTICS_ENV, failure, success} from '../src/results.js';

afterEach(() => {vi.restoreAllMocks(); vi.unstubAllEnvs();});

it.each([1, 65_536])('preserves complete structured content without a mirror (%i Unicode units)', length => {
  const value = {path: '/workspace/result', content: '🧪'.repeat(length), nested: {complete: true}};
  const result = success(value);
  expect(result.content).toEqual([]);
  expect(result.structuredContent).toBe(value);
});

it('leaves the single payload serialization to the MCP transport', () => {
  const stringify = vi.spyOn(JSON, 'stringify');
  const value = {content: 'complete result'};
  const result = success(value);
  expect(stringify).not.toHaveBeenCalled();
  expect(result.structuredContent).toBe(value);
});

it('returns complete unmirrored structured content through the standard MCP SDK', async () => {
  const value = {content: '🧪 complete result\n'.repeat(8192), nested: {complete: true}};
  const server = new McpServer({name: 'result-test', version: '1'});
  server.registerTool('result_probe', {description: 'Return the complete structured result'}, () => success(value));
  const client = new Client({name: 'result-client', version: '1'});
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
  try {
    await server.connect(serverTransport);
    await client.connect(clientTransport);
    const result = await client.callTool({name: 'result_probe'});
    expect(result.isError).not.toBe(true);
    expect(result.content).toEqual([]);
    expect(result.structuredContent).toEqual(value);
  } finally {
    await client.close();
    await server.close();
  }
});

// stdout is the MCP protocol channel, so diagnostics go to stderr. The tool
// result keeps hiding the gateway's body, which may quote credentials.
it('logs the gateway status and body to stderr only when asked, never into the tool result', () => {
  const stderr = vi.spyOn(process.stderr, 'write').mockReturnValue(true);
  const quiet = failure(new HttpError(403, 'denied for token abc123'));
  expect(JSON.stringify(quiet)).not.toContain('abc123');
  expect(stderr).not.toHaveBeenCalled();

  vi.stubEnv(DIAGNOSTICS_ENV, '1');
  const result = failure(new HttpError(403, 'denied for token abc123'));
  expect(JSON.stringify(result)).not.toContain('abc123');
  expect(result.structuredContent).toEqual({error: {kind: 'http', status: 403}});
  const logged = stderr.mock.calls.map(call => String(call[0])).join('');
  expect(logged).toContain('403');
  expect(logged).toContain('denied for token abc123');
});
