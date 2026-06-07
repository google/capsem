// @vitest-environment jsdom

import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProfileCatalogResponse, ProfileListResponse } from '../types/gateway';

let profilesResponse: ProfileListResponse;
let catalogResponse: ProfileCatalogResponse;

const apiMock = {
  listProfiles: vi.fn(async () => profilesResponse),
  getProfileCatalog: vi.fn(async () => catalogResponse),
  selectProfile: vi.fn(async (profileId: string) => {
    profilesResponse = {
      ...profilesResponse,
      default_profile: profileId,
    };
    return {
      mode: 'settings_profiles_v2',
      manifest_present: catalogResponse.manifest_present,
      default_profile: profileId,
      profiles: catalogResponse.profiles,
    };
  }),
};

vi.mock('../api', () => apiMock);

const { default: ProfileCatalogSection } = await import('../components/settings/ProfileCatalogSection.svelte');

function buildProfiles(): ProfileListResponse {
  return {
    mode: 'settings_profiles_v2',
    default_profile: 'everyday-work',
    profiles: [
      {
        source: 'base',
        locked: true,
        profile: {
          id: 'everyday-work',
          name: 'Everyday Work',
          description: 'Balanced defaults for daily work sessions.',
          best_for: 'Daily work with useful tools and measured security prompts.',
          ui: 'everyday',
          revision: '2026.0524.6',
        },
        asset_status: {
          state: 'ready',
          ready: true,
          usable_for_vm: true,
          profile_id: 'everyday-work',
          profile_revision: '2026.0524.6',
          asset_version: 'everyday-work@2026.0524.6',
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
          id: 'coding',
          name: 'Coding',
          description: 'Focused defaults for software development sessions.',
          best_for: 'Coding agents, repository work, tests, and developer tooling.',
          ui: 'coding',
          revision: '2026.0524.6',
        },
        asset_status: {
          state: 'ready',
          ready: true,
          usable_for_vm: true,
          profile_id: 'coding',
          profile_revision: '2026.0524.6',
          asset_version: 'coding@2026.0524.6',
          arch: 'arm64',
          assets: [],
          missing: [],
          missing_assets: [],
        },
      },
    ],
  };
}

function emptyCatalog(): ProfileCatalogResponse {
  return {
    mode: 'settings_profiles_v2',
    manifest_present: false,
    default_profile: 'everyday-work',
    profiles: [],
  };
}

describe('ProfileCatalogSection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    profilesResponse = buildProfiles();
    catalogResponse = emptyCatalog();
  });

  it('renders installed profiles even when no signed catalog manifest is configured', async () => {
    render(ProfileCatalogSection);

    await screen.findByText('Everyday Work');

    expect(screen.getByText('Coding')).toBeTruthy();
    expect(screen.getByText('Default')).toBeTruthy();
    expect(screen.getAllByText('ready').length).toBeGreaterThanOrEqual(2);
    expect(screen.queryByText('No profile catalog installed.')).toBeNull();
    expect(screen.queryByText('No profiles installed.')).toBeNull();
    expect(apiMock.listProfiles).toHaveBeenCalled();
    expect(apiMock.getProfileCatalog).toHaveBeenCalled();
  });

  it('selects a usable installed profile through the profile route', async () => {
    render(ProfileCatalogSection);

    await screen.findByText('Coding');
    const buttons = screen.getAllByRole('button', { name: 'Select' });
    await fireEvent.click(buttons[0]);

    expect(apiMock.selectProfile).toHaveBeenCalledWith('coding');
    await waitFor(() => {
      expect(screen.getByText('Coding selected.')).toBeTruthy();
    });
    expect(apiMock.listProfiles).toHaveBeenCalledTimes(2);
  });

  it('does not allow profiles with missing assets to be selected or leak raw asset paths on cards', async () => {
    profilesResponse = buildProfiles();
    profilesResponse.profiles[1].asset_status = {
      state: 'missing',
      ready: false,
      usable_for_vm: false,
      profile_id: 'coding',
      profile_revision: '2026.0524.6',
      asset_version: 'coding@2026.0524.6',
      arch: 'arm64',
      assets: [],
      missing: ['vmlinuz'],
      missing_assets: [
        {
          name: 'vmlinuz',
          path: '/Users/test/.capsem/assets/arm64/vmlinuz-deadbeef',
          source_url: 'file:///mirror/vmlinuz',
        },
      ],
    };

    render(ProfileCatalogSection);

    await screen.findByText('Coding');
    expect(screen.getByText('assets missing')).toBeTruthy();
    expect(screen.queryByText('/Users/test/.capsem/assets/arm64/vmlinuz-deadbeef')).toBeNull();
    const selectButtons = screen.getAllByRole<HTMLButtonElement>('button', { name: 'Select' });
    expect(selectButtons[0].disabled).toBe(true);
    expect(apiMock.selectProfile).not.toHaveBeenCalled();
  });

  it('refreshes installed profiles on demand', async () => {
    render(ProfileCatalogSection);

    await screen.findByText('Everyday Work');
    profilesResponse = {
      mode: 'settings_profiles_v2',
      default_profile: null,
      profiles: [],
    };

    await fireEvent.click(screen.getByRole('button', { name: 'Refresh profiles' }));

    await waitFor(() => {
      expect(screen.getByText('No profiles installed.')).toBeTruthy();
    });
    expect(apiMock.listProfiles).toHaveBeenCalledTimes(2);
  });
});
