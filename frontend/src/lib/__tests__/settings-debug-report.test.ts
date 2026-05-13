// @vitest-environment jsdom

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { buildMockSettingsResponse } from '../mock-settings';
import type { SettingsResponse } from '../types/settings';

let mockResponse: SettingsResponse;
let debugReportText = '';
const writeText = vi.fn(async (_text: string) => {});

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
Object.defineProperty(navigator, 'clipboard', {
  configurable: true,
  value: { writeText },
});

vi.mock('../api', () => ({
  getSettings: vi.fn(async () => mockResponse),
  saveSettings: vi.fn(async () => mockResponse),
  applyPreset: vi.fn(async () => mockResponse),
  getDebugReport: vi.fn(async () => ({ text: debugReportText })),
  reloadConfig: vi.fn(async () => ({
    success: true,
    reloaded: 0,
    failed_session_count: 0,
    failed_session_ids: [],
    failures: [],
    message: null,
  })),
  ReloadConfigError: class ReloadConfigError extends Error {
    constructor(public result: unknown) {
      super('reload failed');
    }
  },
}));

const { default: SettingsPage } = await import('../components/shell/SettingsPage.svelte');
const { settingsStore } = await import('../stores/settings.svelte');

describe('SettingsPage debug report', () => {
  beforeEach(() => {
    mockResponse = buildMockSettingsResponse();
    debugReportText = 'Capsem Debug Report\ninitrd_manifest_hash: abc123';
    writeText.mockClear();
    settingsStore.model = null;
    settingsStore.loading = false;
    settingsStore.error = null;
    settingsStore.reloadError = null;
    settingsStore.reloadState = null;
  });

  it('copies the pasteable debug report from About', async () => {
    render(SettingsPage);
    await waitFor(() => expect(screen.getAllByText('Appearance').length).toBeGreaterThan(0));

    await fireEvent.click(screen.getByRole('button', { name: 'About' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Copy debug info' }));

    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith(debugReportText);
    });
    expect(screen.getByText('Copied debug info.')).toBeTruthy();
  });
});
