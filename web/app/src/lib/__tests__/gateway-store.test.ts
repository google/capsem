import { afterEach, beforeEach, expect, it, vi } from 'vitest';

vi.mock('../api', () => ({ init: vi.fn(), healthCheck: vi.fn() }));
const api = await import('../api');
const { gatewayStore } = await import('../stores/gateway.svelte');

beforeEach(async () => {
  vi.useFakeTimers();
  vi.mocked(api.init).mockResolvedValue({ connected: true, reachable: true, version: '1', reason: 'ok' });
  vi.mocked(api.healthCheck).mockReset();
  await gatewayStore.init();
});
afterEach(() => {
  gatewayStore.destroy();
  vi.useRealTimers();
});

it('preserves the connection after an invalid probe and keeps polling until recovery', async () => {
  vi.mocked(api.healthCheck).mockRejectedValueOnce(new Error('Invalid gateway status')).mockResolvedValueOnce(true);
  await vi.advanceTimersByTimeAsync(10000);
  expect(gatewayStore.connected).toBe(true);
  expect(gatewayStore.reachable).toBe(true);
  expect(gatewayStore.error).toBe('Invalid gateway status');
  await vi.advanceTimersByTimeAsync(10000);
  expect(api.healthCheck).toHaveBeenCalledTimes(2);
  expect(gatewayStore.error).toBeNull();
});

it('disconnects when the probe confirms connection loss', async () => {
  vi.mocked(api.healthCheck).mockResolvedValueOnce(false);
  await vi.advanceTimersByTimeAsync(10000);
  expect(gatewayStore.connected).toBe(false);
  expect(gatewayStore.reachable).toBe(false);
  expect(gatewayStore.error).toBe('Gateway connection lost');
});
