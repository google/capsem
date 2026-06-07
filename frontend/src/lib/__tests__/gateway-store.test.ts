import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../api', () => ({
  init: vi.fn(),
  healthCheck: vi.fn(),
  getStatus: vi.fn(),
}));

const api = await import('../api');
const { gatewayStore } = await import('../stores/gateway.svelte');

function resetStore() {
  gatewayStore.destroy();
  gatewayStore.connected = false;
  gatewayStore.reachable = false;
  gatewayStore.version = null;
  gatewayStore.error = null;
}

describe('gatewayStore health reconciliation', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetStore();
  });

  it('keeps the app connected when a transient health miss is contradicted by /status', async () => {
    gatewayStore.connected = true;
    gatewayStore.reachable = true;
    gatewayStore.version = 'old';

    vi.mocked(api.healthCheck).mockResolvedValueOnce(false);
    vi.mocked(api.getStatus).mockResolvedValueOnce({
      service: 'running',
      gateway_version: '1.2.3',
      vm_count: 0,
      vms: [],
      resource_summary: null,
    });

    await (gatewayStore as any).doHealthCheck();

    expect(api.healthCheck).toHaveBeenCalledTimes(1);
    expect(api.getStatus).toHaveBeenCalledTimes(1);
    expect(gatewayStore.connected).toBe(true);
    expect(gatewayStore.reachable).toBe(true);
    expect(gatewayStore.version).toBe('1.2.3');
    expect(gatewayStore.error).toBeNull();
  });

  it('marks disconnected only when health and /status both fail', async () => {
    gatewayStore.connected = true;
    gatewayStore.reachable = true;
    gatewayStore.version = 'old';

    vi.mocked(api.healthCheck).mockResolvedValueOnce(false);
    vi.mocked(api.getStatus).mockResolvedValueOnce({
      service: 'offline',
      gateway_version: '',
      vm_count: 0,
      vms: [],
      resource_summary: null,
    });

    await (gatewayStore as any).doHealthCheck();

    expect(gatewayStore.connected).toBe(false);
    expect(gatewayStore.reachable).toBe(false);
    expect(gatewayStore.error).toBe('Gateway connection lost');
  });
});
