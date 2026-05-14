// @vitest-environment jsdom

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import NewTabPage from '../components/shell/NewTabPage.svelte';
import CreateSandboxDialog from '../components/shell/CreateSandboxDialog.svelte';
import { tabStore } from '../stores/tabs.svelte';
import { vmStore } from '../stores/vms.svelte';
import type { AssetHealth, ProvisionRequest } from '../types/gateway';

vi.mock('../api', () => ({
  getStats: vi.fn(async () => ({
    global: {
      total_sessions: 0,
      total_input_tokens: 0,
      total_output_tokens: 0,
      total_estimated_cost: 0,
      total_tool_calls: 0,
      total_mcp_calls: 0,
      total_file_events: 0,
      total_requests: 0,
      total_allowed: 0,
      total_denied: 0,
    },
  })),
}));

const originalProvision = vmStore.provision.bind(vmStore);
const originalOpenVm = tabStore.openVM.bind(tabStore);

function assetHealth(overrides: Partial<AssetHealth>): AssetHealth {
  return {
    ready: false,
    state: 'updating',
    missing: [],
    retry_count: 0,
    retryable: false,
    ...overrides,
  };
}

function resetStores() {
  vmStore.vms = [];
  vmStore.resourceSummary = null;
  vmStore.serviceStatus = 'running';
  vmStore.assetHealth = null;
  vmStore.acting = false;
  vmStore.polled = true;
  vmStore.showCreateModal = false;
  vmStore.error = null;
  vmStore.provision = originalProvision;

  tabStore.tabs = [{ id: 'tab-test', title: 'Dashboard', view: 'new-tab' }];
  tabStore.activeId = 'tab-test';
  tabStore.openVM = originalOpenVm;
}

describe('session runtime truth UI', () => {
  beforeEach(() => {
    resetStores();
  });

  it('treats unknown asset health as not ready', () => {
    render(NewTabPage);

    expect(screen.getByText('VM asset status is unknown')).toBeTruthy();
    expect(screen.getByText('Waiting for the service to report rootfs and manifest readiness.')).toBeTruthy();
    expect((screen.getByRole('button', { name: /quick session/i }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: /customize session/i }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('shows missing asset details and keeps creation disabled', () => {
    vmStore.assetHealth = assetHealth({ state: 'updating', missing: ['rootfs', 'manifest.json'] });
    render(NewTabPage);

    expect(screen.getByText('VM assets are updating')).toBeTruthy();
    expect(screen.getByText('Required VM assets are updating.')).toBeTruthy();
    expect((screen.getByRole('button', { name: /quick session/i }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('quick session lets the service choose resource defaults', async () => {
    const requests: ProvisionRequest[] = [];
    vmStore.assetHealth = assetHealth({ ready: true, state: 'ready', missing: [] });
    vmStore.provision = vi.fn(async (request: ProvisionRequest) => {
      requests.push(request);
      return { id: 'vm-1', name: 'vm-1' };
    });
    tabStore.openVM = vi.fn();

    render(NewTabPage);
    await fireEvent.click(screen.getByRole('button', { name: /quick session/i }));

    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0]).toEqual({ persistent: false });
    expect(tabStore.openVM).toHaveBeenCalledWith('vm-1', 'vm-1');
  });

  it('customize dialog omits CPU and RAM in service-default mode', async () => {
    const requests: ProvisionRequest[] = [];
    vmStore.showCreateModal = true;
    vmStore.provision = vi.fn(async (request: ProvisionRequest) => {
      requests.push(request);
      return { id: 'vm-2', name: 'work' };
    });
    tabStore.openVM = vi.fn();

    render(CreateSandboxDialog);
    await fireEvent.input(screen.getByLabelText(/name/i), { target: { value: 'work' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Create' }));

    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0]).toEqual({ name: 'work', persistent: true });
    expect(tabStore.openVM).toHaveBeenCalledWith('vm-2', 'work');
  });

  it('customize dialog sends explicit resources only in override mode', async () => {
    const requests: ProvisionRequest[] = [];
    vmStore.showCreateModal = true;
    vmStore.provision = vi.fn(async (request: ProvisionRequest) => {
      requests.push(request);
      return { id: 'vm-3', name: 'vm-3' };
    });

    render(CreateSandboxDialog);
    await fireEvent.click(screen.getByRole('button', { name: 'Override' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Create' }));

    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0]).toEqual({ persistent: false, ram_mb: 4096, cpus: 4 });
  });
});
