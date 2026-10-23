import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

import { invoke } from '@tauri-apps/api/core';
import { readGatewayConnection, readGatewayToken } from '../gateway-token';

describe('desktop gateway token', () => {
  afterEach(() => { vi.unstubAllGlobals(); vi.mocked(invoke).mockReset(); });

  it('gets authority through Tauri IPC without an HTTP token request', async () => {
    vi.stubGlobal('window', { __TAURI_INTERNALS__: {} });
    const fetch = vi.fn();
    vi.stubGlobal('fetch', fetch);
    const token = 'a'.repeat(64);
    vi.mocked(invoke).mockResolvedValueOnce({ gatewayUrl: 'http://127.0.0.1:19222', token });

    expect(await readGatewayToken('http://127.0.0.1:19222')).toBe(token);
    expect(invoke).toHaveBeenCalledWith('get_gateway_connection');
    expect(fetch).not.toHaveBeenCalled();
  });

  it('accepts a matching reply from the terminal parent frame', async () => {
    const token = 'b'.repeat(64);
    let listener: ((event: MessageEvent) => void) | undefined;
    const parent = {
      postMessage: vi.fn((request: { requestId: number }, origin: string) => {
        expect(origin).toBe('tauri://localhost');
        queueMicrotask(() => listener?.({
          source: parent, origin: 'https://wrong.example',
          data: { type: 'gateway-token', requestId: request.requestId, token: 'x'.repeat(64), gatewayUrl: 'http://127.0.0.1:65229' },
        } as MessageEvent));
        queueMicrotask(() => listener?.({
          source: {}, origin,
          data: { type: 'gateway-token', requestId: request.requestId, token: 'x'.repeat(64), gatewayUrl: 'http://127.0.0.1:65229' },
        } as MessageEvent));
        queueMicrotask(() => listener?.({
          source: parent,
          origin,
          data: { type: 'gateway-token', requestId: request.requestId, token, gatewayUrl: 'http://127.0.0.1:65228' },
        } as MessageEvent));
      }),
    };
    vi.stubGlobal('window', {
      parent,
      location: { origin: 'tauri://localhost' },
      addEventListener: vi.fn((_type: string, handler: (event: MessageEvent) => void) => { listener = handler; }),
      removeEventListener: vi.fn(),
    });
    const fetch = vi.fn();
    vi.stubGlobal('fetch', fetch);

    expect(await readGatewayConnection('http://127.0.0.1:19222'))
      .toEqual({ gatewayUrl: 'http://127.0.0.1:65228', token });
    expect(parent.postMessage).toHaveBeenCalledOnce();
    expect(fetch).not.toHaveBeenCalled();
  });

  it('refuses refresh authority for a different destination', async () => {
    vi.stubGlobal('window', { __TAURI_INTERNALS__: {} });
    vi.mocked(invoke).mockResolvedValueOnce({ gatewayUrl: 'http://127.0.0.1:65229', token: 'c'.repeat(64) });
    expect(await readGatewayToken('http://127.0.0.1:65228')).toBeNull();
  });
});
