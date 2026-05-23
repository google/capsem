// @vitest-environment jsdom

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import NewTabPage from '../components/shell/NewTabPage.svelte';
import CreateSandboxDialog from '../components/shell/CreateSandboxDialog.svelte';
import { tabStore } from '../stores/tabs.svelte';
import { vmStore } from '../stores/vms.svelte';
import type { AssetHealth, ProvisionRequest } from '../types/gateway';
import * as api from '../api';

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
  retrySetup: vi.fn(async () => undefined),
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
    vi.clearAllMocks();
    resetStores();
  });

  it('treats unknown asset health as not ready', () => {
    render(NewTabPage);

    expect(screen.getByText('VM asset status is unknown')).toBeTruthy();
    expect(screen.getByText('Waiting for the service to report rootfs and manifest readiness.')).toBeTruthy();
    expect((screen.getByRole('button', { name: /quick session/i }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: /customize session/i }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('shows service offline state as a blocking reason', () => {
    vmStore.serviceStatus = 'unavailable';
    vmStore.assetHealth = assetHealth({ ready: true, state: 'ready', missing: [] });
    render(NewTabPage);

    expect(screen.getByText('Capsem service is offline')).toBeTruthy();
    expect(screen.getByText('Start or recover the service before creating sessions.')).toBeTruthy();
    expect((screen.getByRole('button', { name: /quick session/i }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: /customize session/i }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('does not collapse service offline startup failure into empty-session copy', () => {
    vmStore.serviceStatus = 'unavailable';
    vmStore.assetHealth = assetHealth({ ready: true, state: 'ready', missing: [] });
    render(NewTabPage);

    expect(screen.getByText('Capsem service is offline')).toBeTruthy();
    expect(screen.getAllByText('Session list unavailable until startup checks pass')).toHaveLength(2);
    expect(screen.queryByText('No ephemeral sessions')).toBeNull();
    expect(screen.queryByText('No persistent sessions')).toBeNull();
  });

  it('shows missing asset details and keeps creation disabled', () => {
    vmStore.assetHealth = assetHealth({ state: 'updating', missing: ['rootfs', 'manifest.json'] });
    render(NewTabPage);

    expect(screen.getByText('VM assets are updating')).toBeTruthy();
    expect(screen.getByText('Required VM assets are updating.')).toBeTruthy();
    expect((screen.getByRole('button', { name: /quick session/i }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('shows retry setup affordance when service marks asset error as retryable', async () => {
    const refreshSpy = vi.spyOn(vmStore, 'refresh').mockResolvedValue();
    vmStore.assetHealth = assetHealth({ state: 'error', retryable: true, error: 'download failed' });
    render(NewTabPage);

    expect(screen.getByText('VM assets need attention')).toBeTruthy();
    const button = screen.getByRole('button', { name: /retry setup/i });
    await fireEvent.click(button);

    expect(api.retrySetup).toHaveBeenCalledTimes(1);
    expect(refreshSpy).toHaveBeenCalledTimes(1);
    refreshSpy.mockRestore();
  });

  it('surfaces retry setup errors without hiding the refresh affordance', async () => {
    vi.mocked(api.retrySetup).mockRejectedValueOnce(new Error('API error 500: {"error":"asset retry failed"}'));
    const refreshSpy = vi.spyOn(vmStore, 'refresh').mockResolvedValue();
    vmStore.assetHealth = assetHealth({ state: 'error', retryable: true, error: 'download failed' });
    render(NewTabPage);

    await fireEvent.click(screen.getByRole('button', { name: /retry setup/i }));

    expect(await screen.findByText('asset retry failed')).toBeTruthy();
    expect(screen.getByRole('button', { name: /refresh status/i })).toBeTruthy();
    expect(refreshSpy).not.toHaveBeenCalled();
    refreshSpy.mockRestore();
  });

  it('refreshes startup status without requiring a retryable setup error', async () => {
    const refreshSpy = vi.spyOn(vmStore, 'refresh').mockResolvedValue();
    vmStore.assetHealth = assetHealth({ state: 'checking', retryable: false });
    render(NewTabPage);

    await fireEvent.click(screen.getByRole('button', { name: /refresh status/i }));

    expect(refreshSpy).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('button', { name: /retry setup/i })).toBeNull();
    refreshSpy.mockRestore();
  });

  it('quick session lets the service choose resource defaults', async () => {
    const requests: ProvisionRequest[] = [];
    vmStore.assetHealth = assetHealth({
      ready: true,
      state: 'ready',
      missing: [],
      profile_id: 'everyday-work',
      profile_revision: '2026.0520.2',
    });
    vmStore.provision = vi.fn(async (request: ProvisionRequest) => {
      requests.push(request);
      return { id: 'vm-1', name: 'vm-1' };
    });
    tabStore.openVM = vi.fn();

    render(NewTabPage);
    await fireEvent.click(screen.getByRole('button', { name: /quick session/i }));

    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0]).toEqual({
      persistent: false,
      profile_id: 'everyday-work',
      profile_revision: '2026.0520.2',
    });
    expect(tabStore.openVM).toHaveBeenCalledWith('vm-1', 'vm-1');
  });

  it('customize dialog omits CPU and RAM in service-default mode', async () => {
    const requests: ProvisionRequest[] = [];
    vmStore.showCreateModal = true;
    vmStore.assetHealth = assetHealth({
      ready: true,
      state: 'ready',
      missing: [],
      profile_id: 'coding',
      profile_revision: '2026.0520.3',
    });
    vmStore.provision = vi.fn(async (request: ProvisionRequest) => {
      requests.push(request);
      return { id: 'vm-2', name: 'work' };
    });
    tabStore.openVM = vi.fn();

    render(CreateSandboxDialog);
    expect(screen.getByText('coding@2026.0520.3')).toBeTruthy();
    await fireEvent.input(screen.getByLabelText(/name/i), { target: { value: 'work' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Create' }));

    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0]).toEqual({
      name: 'work',
      persistent: true,
      profile_id: 'coding',
      profile_revision: '2026.0520.3',
    });
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
