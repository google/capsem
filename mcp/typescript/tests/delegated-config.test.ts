import {afterEach, describe, expect, it, vi} from 'vitest';
import {parseConfig} from '../src/config.js';

const read = vi.hoisted(() => vi.fn(() => 'operator-file-secret'));
vi.mock('node:fs', async importOriginal => ({...await importOriginal<typeof import('node:fs')>(), readFileSync: read}));
const id = 'fc120782-2272-4b3d-a2e6-a26ee2f8ab61';
const capability = 'af'.repeat(32);
const argv = ['--gateway-url', 'http://127.0.0.1:19222', '--access-id', id];
afterEach(() => read.mockClear());

describe('explicit delegated host MCP configuration', () => {
  it('accepts only the explicit access ID and managed capability environment value', () => {
    const env = new Proxy({CAPSEM_AGENT_CAPABILITY: capability}, {
      get(target, name) {
        if (name !== 'CAPSEM_AGENT_CAPABILITY') throw new Error('operator discovery forbidden');
        return Reflect.get(target, name);
      },
    });
    expect(parseConfig(argv, env)).toEqual({
      gatewayUrl: 'http://127.0.0.1:19222', accessId: id, capability, timeoutMs: 30_000,
    });
    expect(read).not.toHaveBeenCalled();
  });

  it.each(['--token-file', '--token', '--capability', '--token=argv-secret', '--capability=argv-secret'])(
    'refuses credential file or argv input without reading a file or echoing a value', flag => {
      let message = '';
      try {parseConfig([...argv, flag, 'argv-secret'], {CAPSEM_AGENT_CAPABILITY: capability});} catch (error) {
        expect(error).toBeInstanceOf(TypeError);
        message = (error as Error).message;
      }
      expect(message).toMatch(/delegated|CAPSEM_AGENT_CAPABILITY/i);
      expect(message).not.toContain('argv-secret');
      expect(message).not.toContain(capability);
      expect(read).not.toHaveBeenCalled();
    },
  );

  it('never adopts a legacy operator environment value', () => {
    expect(() => parseConfig(argv, {CAPSEM_GATEWAY_TOKEN: 'operator-env-secret'})).toThrow(/capability/i);
    expect(read).not.toHaveBeenCalled();
  });

  it.each(['private-input-secret', 'ff'.repeat(31), 'FF'.repeat(32), `${id}.${capability}`])(
    'rejects malformed capability input without echo', secret => {
      let message = '';
      try {parseConfig(argv, {CAPSEM_AGENT_CAPABILITY: secret});} catch (error) {
        expect(error).toBeInstanceOf(TypeError);
        message = (error as Error).message;
      }
      expect(message).toMatch(/delegated.*capability/i);
      expect(message).not.toContain(secret);
    },
  );
});
