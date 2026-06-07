// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import type { DetectedConfigSummary } from '../types/onboarding';

const { apiMock, state } = vi.hoisted(() => {
  const detection: DetectedConfigSummary = {
    git_name: null,
    git_email: null,
    ssh_public_key_present: false,
    anthropic_api_key_present: false,
    google_api_key_present: false,
    openai_api_key_present: false,
    github_token_present: false,
    claude_oauth_present: false,
    google_adc_present: false,
    settings_written: [],
  };
  const state = {
    settings: null as unknown,
    detection,
    getSettingsFails: true,
  };
  const apiMock = {
    getSettings: vi.fn(async () => {
      if (state.getSettingsFails) throw new Error('settings unavailable');
      return state.settings;
    }),
    runDetection: vi.fn(async () => state.detection),
    saveCredential: vi.fn(async () => ({})),
    saveSettings: vi.fn(async () => ({})),
    validateApiKey: vi.fn(async () => ({ valid: true, message: 'ok' })),
  };
  return { apiMock, state };
});

vi.mock('../api', () => apiMock);

const { default: ProvidersStep } = await import('../components/onboarding/ProvidersStep.svelte');

describe('ProvidersStep', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    state.getSettingsFails = true;
    state.settings = null;
    state.detection = {
      git_name: null,
      git_email: null,
      ssh_public_key_present: false,
      anthropic_api_key_present: false,
      google_api_key_present: false,
      openai_api_key_present: false,
      github_token_present: false,
      claude_oauth_present: false,
      google_adc_present: false,
      settings_written: [],
    };
  });

  it('keeps provider key fields actionable when settings are unavailable', async () => {
    render(ProvidersStep);

    await waitFor(() => {
      expect(screen.getByText('Anthropic')).toBeTruthy();
    });

    expect(screen.getByText('OpenAI')).toBeTruthy();
    expect(screen.getByText('Google AI')).toBeTruthy();
    expect(screen.getByText('GitHub')).toBeTruthy();
    expect(screen.getAllByPlaceholderText('Enter API key...')).toHaveLength(4);
  });

  it('marks Profile V2 service credentials as configured', async () => {
    state.getSettingsFails = false;
    state.settings = {
      mode: 'settings_profiles_v2',
      settings_profiles: {
        service: {
          credential_ids: ['google-api-key', 'github-token'],
        },
      },
      tree: [],
      issues: [],
      presets: [],
    };

    render(ProvidersStep);

    await waitFor(() => {
      expect(screen.getByText('Google AI')).toBeTruthy();
    });

    expect(screen.getAllByText('Configured')).toHaveLength(2);
    expect(screen.getAllByPlaceholderText('Enter API key...')).toHaveLength(2);
  });

  it('saves manually entered keys as Profile V2 credentials', async () => {
    render(ProvidersStep);

    await waitFor(() => {
      expect(screen.getByText('Anthropic')).toBeTruthy();
    });

    const input = screen.getAllByPlaceholderText('Enter API key...')[0];
    await fireEvent.input(input, { target: { value: 'sk-ant-test' } });
    await fireEvent.click(screen.getAllByText('Validate')[0]);

    await waitFor(() => {
      expect(apiMock.saveCredential).toHaveBeenCalledWith(
        'anthropic-api-key',
        'sk-ant-test',
        'Anthropic API key',
      );
    });
    expect(apiMock.saveSettings).not.toHaveBeenCalled();
  });
});
