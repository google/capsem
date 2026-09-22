import { describe, expect, it } from 'vitest';
import { ApiError, GATEWAY_REQUEST_BUDGET_MS, GatewaySdk } from '../gateway-sdk';

const connection = {
  url: () => 'http://127.0.0.1:19222',
  token: () => 'token',
  refreshToken: async () => false,
};

describe('GatewaySdk deadlines', () => {
  // Fork of a large VM or Apply Update take longer than the SDK's 30 s
  // default; the gateway itself waits 120 s for them. The UI must not give
  // up first while the operation keeps running.
  it('waits at least as long as the gateway does', async () => {
    let seen = 0;
    await new GatewaySdk(connection).call(async transport => {
      seen = transport.timeoutMs;
    });
    expect(GATEWAY_REQUEST_BUDGET_MS).toBeGreaterThanOrEqual(120_000);
    expect(seen).toBe(GATEWAY_REQUEST_BUDGET_MS);
  });

  it('reports a client deadline as an API error, not a bare DOMException', async () => {
    const call = new GatewaySdk(connection).call(async () => {
      throw new DOMException('The operation timed out.', 'TimeoutError');
    });
    await expect(call).rejects.toBeInstanceOf(ApiError);
    await expect(call).rejects.toMatchObject({ status: 504 });
    await expect(call).rejects.toThrow(/may still be running/);
  });

  it('passes other failures through unchanged', async () => {
    const failure = new DOMException('stopped', 'AbortError');
    await expect(new GatewaySdk(connection).call(async () => { throw failure; })).rejects.toBe(failure);
  });
});
