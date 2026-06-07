// @vitest-environment jsdom

import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProfileListResponse } from '../types/gateway';

let profilesResponse: ProfileListResponse;

Object.defineProperty(window, 'matchMedia', {
  writable: true,
  value: vi.fn().mockImplementation((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })),
});

const apiMock = {
  listProfiles: vi.fn(async () => profilesResponse),
  selectProfile: vi.fn(async (profileId: string) => {
    profilesResponse = { ...profilesResponse, default_profile: profileId };
    return {
      mode: 'settings_profiles_v2',
      manifest_present: false,
      default_profile: profileId,
      profiles: [],
    };
  }),
  getSettings: vi.fn(async () => ({ tree: [], issues: [], presets: [] })),
  saveSettings: vi.fn(async () => ({ tree: [], issues: [], presets: [] })),
};

vi.mock('../api', () => apiMock);

const { default: PreferencesStep } = await import('../components/onboarding/PreferencesStep.svelte');

function buildProfilesResponse(): ProfileListResponse {
  return {
    mode: 'settings_profiles_v2',
    default_profile: 'coding',
    profiles: [
      {
        source: 'base',
        locked: true,
        profile: {
          id: 'coding',
          name: 'Coding',
          revision: '2026.0520.1',
        },
        asset_status: {
          state: 'ready',
          ready: true,
          usable_for_vm: true,
          profile_id: 'coding',
          profile_revision: '2026.0520.1',
          asset_version: 'coding@2026.0520.1',
          arch: 'arm64',
          assets: [],
          missing: [],
          missing_assets: [],
        },
      },
      {
        source: 'base',
        locked: true,
        profile: {
          id: 'everyday-work',
          name: 'Everyday Work',
          revision: '2026.0520.1',
        },
        asset_status: {
          state: 'ready',
          ready: true,
          usable_for_vm: true,
          profile_id: 'everyday-work',
          profile_revision: '2026.0520.1',
          asset_version: 'everyday-work@2026.0520.1',
          arch: 'arm64',
          assets: [],
          missing: [],
          missing_assets: [],
        },
      },
      {
        source: 'base',
        locked: true,
        profile: {
          id: 'broken-profile',
          name: 'Broken Profile',
          revision: '2026.0520.1',
        },
        asset_status: {
          state: 'missing',
          ready: false,
          usable_for_vm: false,
          profile_id: 'broken-profile',
          profile_revision: '2026.0520.1',
          asset_version: 'broken-profile@2026.0520.1',
          arch: 'arm64',
          assets: [],
          missing: ['vmlinuz'],
          missing_assets: [],
        },
      },
    ],
  };
}

describe('PreferencesStep', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    profilesResponse = buildProfilesResponse();
  });

  it('selects onboarding profiles through the Profile V2 catalog route', async () => {
    render(PreferencesStep);

    await screen.findByText('Profile');
    const profileSelect = screen.getAllByRole<HTMLSelectElement>('combobox')[0];
    expect(profileSelect.value).toBe('coding');
    expect(screen.getByText('Profile')).toBeTruthy();
    expect(screen.queryByText('Security Preset')).toBeNull();
    expect(apiMock.listProfiles).toHaveBeenCalled();

    await fireEvent.change(profileSelect, { target: { value: 'everyday-work' } });

    await waitFor(() => {
      expect(apiMock.selectProfile).toHaveBeenCalledWith('everyday-work');
    });
    expect(apiMock.listProfiles).toHaveBeenCalledTimes(2);
  });

  it('does not offer profiles with unusable assets as selectable wizard choices', async () => {
    render(PreferencesStep);

    await screen.findByText('Profile');
    const option = screen.getByRole<HTMLOptionElement>('option', {
      name: 'Broken Profile@2026.0520.1',
    });
    expect(option.disabled).toBe(true);
  });

  it('shows agent-friendly VM defaults without exposing stale settings controls', async () => {
    render(PreferencesStep);

    await screen.findByText('Profile');
    expect(screen.getByText('CPU cores')).toBeTruthy();
    expect(screen.getByText('RAM')).toBeTruthy();
    expect(screen.getByText('Active VMs')).toBeTruthy();
    expect(screen.getByText('8 GB')).toBeTruthy();
    expect(screen.getAllByText('8').length).toBeGreaterThanOrEqual(1);
    expect(apiMock.saveSettings).not.toHaveBeenCalled();
  });
});
