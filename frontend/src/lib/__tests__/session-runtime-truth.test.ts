// @vitest-environment jsdom

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import NewTabPage from '../components/shell/NewTabPage.svelte';
import CreateSandboxDialog from '../components/shell/CreateSandboxDialog.svelte';
import { gatewayStore } from '../stores/gateway.svelte';
import { tabStore } from '../stores/tabs.svelte';
import { vmStore } from '../stores/vms.svelte';
import type { AssetHealth, ProvisionRequest } from '../types/gateway';
import * as api from '../api';

const mockApiState = vi.hoisted(() => ({
  status: {
    service: 'running',
    gateway_version: '0.1',
    vm_count: 0,
    vms: [],
    resource_summary: null,
    assets: null,
  } as any,
}));

vi.mock('../api', () => ({
  init: vi.fn(async () => ({ connected: true, reachable: true, version: 'test' })),
  healthCheck: vi.fn(async () => true),
  getStatus: vi.fn(async () => mockApiState.status),
  getSetupState: vi.fn(async () => ({ needs_onboarding: false, install_completed: true })),
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
  openUrl: vi.fn(async () => undefined),
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
  gatewayStore.destroy();
  gatewayStore.connected = true;
  gatewayStore.reachable = true;
  gatewayStore.version = 'test';
  gatewayStore.error = null;

  vmStore.stopPolling();
  vmStore.vms = [];
  vmStore.resourceSummary = null;
  vmStore.serviceStatus = 'running';
  vmStore.assetHealth = null;
  vmStore.acting = false;
  vmStore.polled = true;
  vmStore.showCreateModal = false;
  vmStore.showAssetReadinessModal = false;
  vmStore.error = null;
  vmStore.provision = originalProvision;
  mockApiState.status = {
    service: 'running',
    gateway_version: '0.1',
    vm_count: 0,
    vms: [],
    resource_summary: null,
    assets: null,
  };

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
    expect((screen.getByRole('button', { name: /quick session/i }) as HTMLButtonElement).disabled).toBe(false);
    expect((screen.getByRole('button', { name: /customize session/i }) as HTMLButtonElement).disabled).toBe(false);
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

  it('renders VM profile identity and marks missing profile pins as corrupted', () => {
    vmStore.assetHealth = assetHealth({ ready: true, state: 'ready', missing: [] });
    vmStore.vms = [
      {
        id: 'vm-current',
        name: 'Current VM',
        status: 'Running',
        persistent: false,
        profile_id: 'coding',
        profile_revision: '2026.0520.3',
        profile_status: 'current',
      },
      {
        id: 'vm-drift',
        name: 'Needs Update VM',
        status: 'Stopped',
        persistent: false,
        profile_id: 'everyday-work',
        profile_revision: '2026.0520.1',
        profile_status: 'needs_update',
      },
      {
        id: 'vm-missing',
        name: 'Missing Profile VM',
        status: 'Stopped',
        persistent: false,
      },
    ];

    render(NewTabPage);

    expect(screen.getByText('coding@2026.0520.3')).toBeTruthy();
    expect(screen.getByText('everyday-work@2026.0520.1')).toBeTruthy();
    expect(screen.getByText('missing profile')).toBeTruthy();
    expect(screen.getByText('current')).toBeTruthy();
    expect(screen.getByText('needs update')).toBeTruthy();
    expect(screen.getByText('corrupted')).toBeTruthy();
  });

  it('shows ready profile asset provenance before session creation', () => {
    vmStore.assetHealth = assetHealth({
      ready: true,
      state: 'ready',
      missing: [],
      version: '2026.0520.2',
      arch: 'arm64',
      profile_id: 'coding',
      profile_revision: '2026.0520.3',
      profile_payload_hash: `blake3:${'e'.repeat(64)}`,
      profile_assets: [
        {
          logical_name: 'vmlinuz',
          hash: `blake3:${'a'.repeat(64)}`,
          source_url: 'https://assets.example.test/coding/arm64/vmlinuz',
          size: 12 * 1024,
          content_type: 'application/octet-stream',
        },
        {
          logical_name: 'rootfs',
          hash: `blake3:${'b'.repeat(64)}`,
          source_url: 'https://assets.example.test/coding/arm64/rootfs',
          size: 5 * 1024 * 1024,
          content_type: 'application/octet-stream',
        },
      ],
    });

    render(NewTabPage);

    expect(screen.getByText('Profile Assets')).toBeTruthy();
    expect(screen.getByText('coding@2026.0520.3')).toBeTruthy();
    expect(screen.getByText('arm64')).toBeTruthy();
    expect(screen.getByText('2026.0520.2')).toBeTruthy();
    expect(screen.getByText('vmlinuz')).toBeTruthy();
    expect(screen.getByText('rootfs')).toBeTruthy();
    expect(screen.getByText('12.0 KB')).toBeTruthy();
    expect(screen.getByText('5.0 MB')).toBeTruthy();
  });

  it('shows missing asset details and download progress without disabling launch controls', () => {
    vmStore.assetHealth = assetHealth({
      state: 'updating',
      missing: ['rootfs', 'manifest.json'],
      progress: {
        logical_name: 'rootfs',
        bytes_done: 25 * 1024 * 1024,
        bytes_total: 100 * 1024 * 1024,
        done: false,
      },
    });
    render(NewTabPage);

    expect(screen.getByText('VM assets are updating')).toBeTruthy();
    expect(screen.getByText('Updating rootfs.')).toBeTruthy();
    expect(screen.getByText('Missing: rootfs, manifest.json')).toBeTruthy();
    expect(screen.getByRole('progressbar', { name: /profile asset download progress/i }).getAttribute('aria-valuenow')).toBe('25');
    expect((screen.getByRole('button', { name: /quick session/i }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('checks asset status at first launch and opens a progress modal instead of provisioning too early', async () => {
    const refreshSpy = vi.spyOn(vmStore, 'refresh').mockResolvedValue();
    vmStore.assetHealth = assetHealth({
      state: 'updating',
      profile_id: 'coding',
      profile_revision: '2026.0520.3',
      progress: {
        logical_name: 'rootfs',
        bytes_done: 40 * 1024 * 1024,
        bytes_total: 100 * 1024 * 1024,
        done: false,
      },
    });
    vmStore.provision = vi.fn(async () => ({ id: 'vm-blocked', name: 'vm-blocked' }));

    render(NewTabPage);
    await fireEvent.click(screen.getByRole('button', { name: /quick session/i }));

    expect(refreshSpy).toHaveBeenCalledTimes(1);
    expect(screen.getByRole('dialog', { name: /preparing profile assets/i })).toBeTruthy();
    expect(screen.getAllByText('coding@2026.0520.3')).toHaveLength(2);
    expect(screen.getAllByRole('progressbar', { name: /profile asset download progress/i })).toHaveLength(2);
    expect(vmStore.provision).not.toHaveBeenCalled();
    refreshSpy.mockRestore();
  });

  it('global new-session shortcut uses the same asset readiness gate', async () => {
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: vi.fn(() => ({
        matches: false,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      })),
    });
    const { default: App } = await import('../components/shell/App.svelte');
    const updatingAssets = assetHealth({
      state: 'updating',
      profile_id: 'coding',
      profile_revision: '2026.0520.3',
      progress: {
        logical_name: 'rootfs',
        bytes_done: 40 * 1024 * 1024,
        bytes_total: 100 * 1024 * 1024,
        done: false,
      },
    });
    mockApiState.status = {
      service: 'running',
      gateway_version: '0.1',
      vm_count: 0,
      vms: [],
      resource_summary: null,
      assets: updatingAssets,
    };
    vmStore.provision = vi.fn(async () => ({ id: 'vm-shortcut', name: 'vm-shortcut' }));

    render(App);
    await waitFor(() => expect(screen.getByText('Sessions')).toBeTruthy());
    await fireEvent.keyDown(window, { key: 'n', metaKey: true });

    expect(await screen.findByRole('dialog', { name: /preparing profile assets/i })).toBeTruthy();
    expect(screen.getAllByText('coding@2026.0520.3')).toHaveLength(2);
    expect(vmStore.provision).not.toHaveBeenCalled();
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
    const refreshSpy = vi.spyOn(vmStore, 'refresh').mockResolvedValue();
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
    expect(refreshSpy).toHaveBeenCalled();
    refreshSpy.mockRestore();
  });

  it('customize dialog omits CPU and RAM in service-default mode', async () => {
    const requests: ProvisionRequest[] = [];
    const refreshSpy = vi.spyOn(vmStore, 'refresh').mockResolvedValue();
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
    expect(refreshSpy).toHaveBeenCalled();
    refreshSpy.mockRestore();
  });

  it('customize dialog sends explicit resources only in override mode', async () => {
    const requests: ProvisionRequest[] = [];
    const refreshSpy = vi.spyOn(vmStore, 'refresh').mockResolvedValue();
    vmStore.showCreateModal = true;
    vmStore.assetHealth = assetHealth({ ready: true, state: 'ready', missing: [] });
    vmStore.provision = vi.fn(async (request: ProvisionRequest) => {
      requests.push(request);
      return { id: 'vm-3', name: 'vm-3' };
    });

    render(CreateSandboxDialog);
    await fireEvent.click(screen.getByRole('button', { name: 'Override' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Create' }));

    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0]).toEqual({ persistent: false, ram_mb: 4096, cpus: 4 });
    expect(refreshSpy).toHaveBeenCalled();
    refreshSpy.mockRestore();
  });
});
