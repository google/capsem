import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

// Mock api so store imports succeed.
vi.mock('../api', () => ({
  getStatus: vi.fn(async () => ({ service: 'running', gateway_version: '0.1', vm_count: 0, vms: [], resource_summary: null })),
  provisionVm: vi.fn(),
}));

const { tabStore } = await import('../stores/tabs.svelte');

describe('URL deep-linking', () => {
  const mockReplaceState = vi.fn();

  beforeEach(() => {
    mockReplaceState.mockClear();
    vi.stubGlobal('history', { replaceState: mockReplaceState });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  function simulateUrlParams(search: string) {
    const params = new URLSearchParams(search);
    const connectId = params.get('connect');

    if (connectId) {
      tabStore.openVM(connectId, connectId);
      history.replaceState(null, '', '/');
    }
  }

  it('?connect=abc123 opens VM terminal tab', () => {
    simulateUrlParams('?connect=abc123');

    const tab = tabStore.tabs.find(t => t.vmId === 'abc123');
    expect(tab).toBeDefined();
    expect(tab?.view).toBe('terminal');
    expect(mockReplaceState).toHaveBeenCalled();
  });

  it('no params does nothing', () => {
    simulateUrlParams('');

    expect(mockReplaceState).not.toHaveBeenCalled();
  });

  it('clears search params after handling', () => {
    simulateUrlParams('?connect=vm-x');
    expect(mockReplaceState).toHaveBeenCalledWith(null, '', '/');
  });
});

describe('__capsemDeepLink handler', () => {
  // Simulate the handler that App.svelte registers on window.
  function deepLink(p: { connect?: string }) {
    if (p.connect) {
      tabStore.openVM(p.connect, p.connect);
    }
  }

  it('connect opens VM tab', () => {
    deepLink({ connect: 'vm-deep' });

    const tab = tabStore.tabs.find(t => t.vmId === 'vm-deep');
    expect(tab).toBeDefined();
    expect(tab?.view).toBe('terminal');
  });

  it('empty params does nothing', () => {
    const before = tabStore.tabs.length;
    deepLink({});
    expect(tabStore.tabs.length).toBe(before);
  });
});
