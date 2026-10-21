import { describe, expect, it, vi, afterEach } from 'vitest';
import { gatewayBaseUrl } from '../gateway-url';
import { streamUrl } from '../terminal/stream-protocol';

afterEach(() => vi.unstubAllGlobals());

describe('shared gateway origin', () => {
  it.each(['http://127.0.0.1:49539', 'http://localhost:49539', 'http://127.0.0.1:19222'])(
    'keeps HTTP and terminal connections on the served origin %s', origin => {
      vi.stubGlobal('window', { location: new URL(origin) });
      const http = gatewayBaseUrl();
      expect(http).toBe(origin);
      expect(streamUrl(http.replace(/^http/, 'ws'), 'owned-vm', 'test-token'))
        .toBe(origin.replace(/^http/, 'ws') + '/vms/owned-vm/stream?token=test-token');
    },
  );

  it.each(['http://localhost:5173', 'http://127.0.0.1:5173', 'tauri://localhost', 'https://tauri.localhost'])(
    'uses the installed service from the development or desktop shell %s', origin => {
      vi.stubGlobal('window', { location: new URL(origin) });
      expect(gatewayBaseUrl()).toBe('http://127.0.0.1:19222');
    },
  );

  it('uses the default during server-side rendering', () => {
    vi.stubGlobal('window', undefined);
    expect(gatewayBaseUrl()).toBe('http://127.0.0.1:19222');
  });

  it.each(['http://localhost.example.com:49539', 'http://127.0.0.1.example.com:49539', 'https://example.com'])(
    'does not treat a non-loopback origin as the local gateway: %s', origin => {
      vi.stubGlobal('window', { location: new URL(origin) });
      expect(gatewayBaseUrl()).toBe('http://127.0.0.1:19222');
    },
  );
});
