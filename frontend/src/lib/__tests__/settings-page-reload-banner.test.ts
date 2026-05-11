// @vitest-environment jsdom

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { buildMockSettingsResponse } from '../mock-settings';
import type { SettingsResponse } from '../types/settings';
import type { VmSummary } from '../types/gateway';

let mockResponse: SettingsResponse;
let reloadCalls = 0;

vi.stubGlobal('matchMedia', vi.fn((query: string) => ({
  matches: false,
  media: query,
  onchange: null,
  addEventListener: vi.fn(),
  removeEventListener: vi.fn(),
  addListener: vi.fn(),
  removeListener: vi.fn(),
  dispatchEvent: vi.fn(),
})));
vi.stubGlobal('__APP_VERSION__', 'test');

vi.mock('../api', () => ({
  getSettings: vi.fn(async () => mockResponse),
  saveSettings: vi.fn(async () => mockResponse),
  applyPreset: vi.fn(async () => mockResponse),
  reloadConfig: vi.fn(async () => {
    reloadCalls += 1;
    return {
      success: true,
      reloaded: 1,
      failed_session_count: 0,
      failed_session_ids: [],
      failures: [],
      message: null,
    };
  }),
  ReloadConfigError: class ReloadConfigError extends Error {
    constructor(public result: unknown) {
      super('reload failed');
    }
  },
}));

const { default: SettingsPage } = await import('../components/shell/SettingsPage.svelte');
const { settingsStore } = await import('../stores/settings.svelte');
const { vmStore } = await import('../stores/vms.svelte');

function vm(id: string, status: string): VmSummary {
  return {
    id,
    name: id,
    status,
    persistent: false,
  };
}

async function renderLoadedSettingsPage() {
  render(SettingsPage);
  await waitFor(() => expect(screen.getAllByText('Appearance').length).toBeGreaterThan(0));
}

async function setReloadFailure(ids: string[]) {
  settingsStore.reloadState = {
    persisted: true,
    applied: false,
    failed_session_count: ids.length,
    failed_session_ids: ids,
    message: `failed to reload config in ${ids.length} running session(s)`,
    retry_available: true,
  };
  settingsStore.reloadError = `Saved, but the running service did not reload: ${settingsStore.reloadState.message}`;
  await tick();
}

describe('SettingsPage reload failure banner', () => {
  beforeEach(() => {
    mockResponse = buildMockSettingsResponse();
    reloadCalls = 0;
    settingsStore.model = null;
    settingsStore.loading = false;
    settingsStore.error = null;
    settingsStore.reloadError = null;
    settingsStore.reloadState = null;
    vmStore.vms = [];
  });

  it('shows affected sessions and retries the runtime reload', async () => {
    await renderLoadedSettingsPage();
    vmStore.vms = [vm('vm-a', 'Running'), vm('vm-b', 'Booting')];
    await setReloadFailure(['vm-a', 'vm-b']);

    expect(screen.getByText(/Saved, but the running service did not reload/)).toBeTruthy();
    expect(screen.getByText('Affected sessions: vm-a, vm-b')).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: 'Retry reload' }));

    await waitFor(() => expect(reloadCalls).toBe(1));
    expect(screen.queryByText(/Saved, but the running service did not reload/)).toBeNull();
  });

  it('dismisses the banner when every affected session stops', async () => {
    await renderLoadedSettingsPage();
    vmStore.vms = [vm('vm-a', 'Running')];
    await setReloadFailure(['vm-a']);
    expect(screen.getByText('Affected sessions: vm-a')).toBeTruthy();

    vmStore.vms = [vm('vm-a', 'Stopped')];
    await tick();

    await waitFor(() => {
      expect(screen.queryByText(/Saved, but the running service did not reload/)).toBeNull();
    });
  });
});
