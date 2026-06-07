// @vitest-environment jsdom

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import NewTabPage from '../components/shell/NewTabPage.svelte';
import CreateSandboxDialog from '../components/shell/CreateSandboxDialog.svelte';
import Toolbar from '../components/shell/Toolbar.svelte';
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
  listProfiles: vi.fn(async () => ({
    mode: 'settings_profiles_v2',
    default_profile: 'coding',
    profiles: [
      {
        source: 'base',
        locked: true,
        profile: {
          id: 'coding',
          name: 'Coding',
          description: 'Focused defaults for software development sessions.',
          best_for: 'Coding agents, repository work, tests, and developer tooling.',
          ui: 'coding',
          revision: '2026.0520.3',
        },
        asset_status: {
          state: 'ready',
          ready: true,
          usable_for_vm: true,
          profile_id: 'coding',
          profile_revision: '2026.0520.3',
          asset_version: 'coding@2026.0520.3',
          arch: 'arm64',
          assets: [],
          missing: [],
          missing_assets: [],
        },
      },
    ],
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

  it('treats unknown asset health as not ready without hiding profile launch cards', async () => {
    render(NewTabPage);

    expect(screen.getByText('VM asset status is unknown')).toBeTruthy();
    expect(screen.getByText('Waiting for the service to report rootfs and manifest readiness.')).toBeTruthy();
    expect(await screen.findByText('Coding')).toBeTruthy();
    expect((screen.getByRole('button', { name: /start session/i }) as HTMLButtonElement).disabled).toBe(false);
    expect((screen.getByRole('button', { name: /advanced/i }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('shows service offline state as a blocking reason', async () => {
    vmStore.serviceStatus = 'unavailable';
    vmStore.assetHealth = assetHealth({ ready: true, state: 'ready', missing: [] });
    render(NewTabPage);

    expect(screen.getByText('Capsem service is offline')).toBeTruthy();
    expect(screen.getByText('Start or recover the service before creating sessions.')).toBeTruthy();
    await screen.findByText('Coding');
    expect((screen.getByRole('button', { name: /start session/i }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: /advanced/i }) as HTMLButtonElement).disabled).toBe(true);
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

  it('renders live VM token and cost counters in the toolbar', () => {
    tabStore.tabs = [{ id: 'tab-vm', title: 'Session', view: 'terminal', vmId: 'vm-live' }];
    tabStore.activeId = 'tab-vm';
    vmStore.vms = [
      {
        id: 'vm-live',
        name: null,
        status: 'Running',
        persistent: false,
        total_input_tokens: 1200,
        total_output_tokens: 345,
        total_estimated_cost: 0.42,
        total_tool_calls: 7,
      },
    ];

    render(Toolbar);

    expect(screen.getByTitle('Tokens').textContent).toBe('1.5K tok');
    expect(screen.getByTitle('Tool calls').textContent).toBe('7 calls');
    expect(screen.getByTitle('Cost').textContent).toBe('$0.42');
  });

  it('shows installed profile cards instead of raw asset provenance before session creation', async () => {
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

    expect(await screen.findByText('Coding')).toBeTruthy();
    expect(screen.getByText('Focused defaults for software development sessions.')).toBeTruthy();
    expect(screen.getByText('2026.0520.3')).toBeTruthy();
    expect(screen.getByRole('button', { name: /start session/i })).toBeTruthy();
    expect(screen.queryByText('Profile Assets')).toBeNull();
    expect(screen.queryByText('vmlinuz')).toBeNull();
    expect(screen.queryByText('rootfs')).toBeNull();
  });

  it('shows missing asset details and download progress without disabling launch controls', async () => {
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
    expect(await screen.findByRole('button', { name: /start session/i })).toBeTruthy();
  });

  it('starts a session from the clicked profile card', async () => {
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
    const requests: ProvisionRequest[] = [];
    vmStore.provision = vi.fn(async (request: ProvisionRequest) => {
      requests.push(request);
      return { id: 'vm-profile', name: 'vm-profile' };
    });
    tabStore.openVM = vi.fn();

    render(NewTabPage);
    await fireEvent.click(await screen.findByRole('button', { name: /start session/i }));

    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0]).toEqual({
      persistent: false,
      profile_id: 'coding',
      profile_revision: '2026.0520.3',
    });
    expect(tabStore.openVM).toHaveBeenCalledWith('vm-profile', 'vm-profile');
  });

  it('refuses to launch when a profile card has missing assets', async () => {
    vi.mocked(api.listProfiles).mockResolvedValueOnce({
      mode: 'settings_profiles_v2',
      default_profile: 'broken-profile',
      profiles: [
        {
          source: 'base',
          locked: true,
          profile: {
            id: 'broken-profile',
            name: 'Broken Profile',
            description: 'Broken test profile.',
            best_for: 'Nothing until assets are fixed.',
            ui: 'coding',
            revision: '2026.0520.3',
          },
          asset_status: {
            state: 'missing',
            ready: false,
            usable_for_vm: false,
            profile_id: 'broken-profile',
            profile_revision: '2026.0520.3',
            asset_version: 'broken-profile@2026.0520.3',
            arch: 'arm64',
            assets: [],
            missing: ['vmlinuz'],
            missing_assets: [],
          },
        },
      ],
    });
    vmStore.assetHealth = assetHealth({
      state: 'error',
      ready: false,
      profile_id: 'broken-profile',
      profile_revision: '2026.0520.3',
      missing: ['vmlinuz'],
      error: 'selected profile VM assets are not ready',
    });
    vmStore.provision = vi.fn(async () => ({ id: 'vm-bad-profile', name: 'vm-bad-profile' }));

    render(NewTabPage);

    expect(await screen.findByText('Broken Profile')).toBeTruthy();
    expect(screen.getByText('Assets missing')).toBeTruthy();
    expect((screen.getByRole('button', { name: /start session/i }) as HTMLButtonElement).disabled).toBe(true);
    expect(vmStore.provision).not.toHaveBeenCalled();
  });

  it('refuses to launch a profile that has assets but no signed catalog revision', async () => {
    vi.mocked(api.listProfiles).mockResolvedValueOnce({
      mode: 'settings_profiles_v2',
      default_profile: 'coding',
      profiles: [
        {
          source: 'base',
          locked: true,
          profile: {
            id: 'coding',
            name: 'Coding',
            description: 'Focused defaults for software development sessions.',
            best_for: 'Coding agents, repository work, tests, and developer tooling.',
            ui: 'coding',
            revision: null,
          },
          asset_status: {
            state: 'error',
            ready: false,
            usable_for_vm: false,
            profile_id: 'coding',
            profile_revision: null,
            profile_payload_hash: null,
            asset_version: 'coding',
            arch: 'arm64',
            assets: [
              {
                name: 'vmlinuz',
                path: '/Users/test/.capsem/assets/vmlinuz-good',
                status: 'present',
                source_url: 'file:///mirror/vmlinuz',
              },
            ],
            missing: [],
            missing_assets: [],
            error: "profile 'coding' has no installed signed catalog revision; install it before creating a VM",
          },
        },
      ],
    });
    vmStore.assetHealth = assetHealth({ ready: true, state: 'ready', missing: [] });
    vmStore.provision = vi.fn(async () => ({ id: 'vm-unsigned-profile', name: 'vm-unsigned-profile' }));

    render(NewTabPage);

    expect(await screen.findByText('Coding')).toBeTruthy();
    expect(screen.getByText('Unavailable')).toBeTruthy();
    expect(screen.queryByText('/Users/test/.capsem/assets/vmlinuz-good')).toBeNull();
    expect((screen.getByRole('button', { name: /start session/i }) as HTMLButtonElement).disabled).toBe(true);
    expect(vmStore.provision).not.toHaveBeenCalled();
  });

  it('keeps advanced create disabled when the selected profile has missing assets', async () => {
    vi.mocked(api.listProfiles).mockResolvedValueOnce({
      mode: 'settings_profiles_v2',
      default_profile: 'broken-profile',
      profiles: [
        {
          source: 'base',
          locked: true,
          profile: {
            id: 'broken-profile',
            name: 'Broken Profile',
            description: 'Broken test profile.',
            best_for: 'Nothing until assets are fixed.',
            ui: 'coding',
            revision: '2026.0520.3',
          },
          asset_status: {
            state: 'missing',
            ready: false,
            usable_for_vm: false,
            profile_id: 'broken-profile',
            profile_revision: '2026.0520.3',
            asset_version: 'broken-profile@2026.0520.3',
            arch: 'arm64',
            assets: [],
            missing: ['rootfs.squashfs'],
            missing_assets: [],
          },
        },
      ],
    });
    vmStore.showCreateModal = true;
    vmStore.assetHealth = assetHealth({
      state: 'error',
      ready: false,
      profile_id: 'broken-profile',
      profile_revision: '2026.0520.3',
      missing: ['rootfs.squashfs'],
      error: 'selected profile VM assets are not ready',
    });

    render(CreateSandboxDialog);

    expect(await screen.findByText('Broken Profile')).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Create' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText('Assets missing')).toBeTruthy();
  });

  it('global new-session shortcut opens the profile-based advanced dialog', async () => {
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

    expect(await screen.findByRole('dialog', { name: /new session/i })).toBeTruthy();
    expect((await screen.findAllByText('Coding')).length).toBeGreaterThan(0);
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

  it('profile card session lets the service choose resource defaults', async () => {
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
    await fireEvent.click(await screen.findByRole('button', { name: /start session/i }));

    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0]).toEqual({
      persistent: false,
      profile_id: 'coding',
      profile_revision: '2026.0520.3',
    });
    expect(tabStore.openVM).toHaveBeenCalledWith('vm-1', 'vm-1');
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
    expect(await screen.findByText('Coding')).toBeTruthy();
    expect(screen.getByText('2026.0520.3')).toBeTruthy();
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
    await screen.findByText('Coding');
    await fireEvent.click(screen.getByRole('button', { name: 'Override' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Create' }));

    await waitFor(() => expect(requests).toHaveLength(1));
    expect(requests[0]).toEqual({
      persistent: false,
      profile_id: 'coding',
      profile_revision: '2026.0520.3',
      ram_mb: 8192,
      cpus: 4,
    });
    expect(refreshSpy).toHaveBeenCalled();
    refreshSpy.mockRestore();
  });
});
