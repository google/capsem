import { describe, expect, it } from 'vitest';
import { parseGatewayConnection } from '../gateway-connection';

describe('private gateway connection', () => {
  const token = 'a'.repeat(64);
  it('keeps a selected loopback port paired with its authority', () => {
    expect(parseGatewayConnection({ gatewayUrl: 'http://127.0.0.1:65228', token }))
      .toEqual({ gatewayUrl: 'http://127.0.0.1:65228', token });
  });
  it.each([
    'https://remote.example', 'http://127.0.0.1.example:65228',
    'http://user@127.0.0.1:65228', 'http://127.0.0.1:65228/path',
    'http://127.0.0.1:65228?token=x', 'http://127.0.0.1:0',
  ])('rejects non-origin or non-loopback authority %s', gatewayUrl => {
    expect(parseGatewayConnection({ gatewayUrl, token })).toBeNull();
  });
  it('rejects incomplete authority', () => {
    expect(parseGatewayConnection({ gatewayUrl: 'http://127.0.0.1:65228', token: 'short' })).toBeNull();
  });
});
