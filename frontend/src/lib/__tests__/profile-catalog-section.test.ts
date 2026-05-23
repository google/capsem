// @vitest-environment jsdom

import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProfileCatalogResponse } from '../types/gateway';

let catalog: ProfileCatalogResponse;

const apiMock = {
  getProfileCatalog: vi.fn(async () => catalog),
  selectProfile: vi.fn(async (profileId: string) => {
    catalog = {
      ...catalog,
      default_profile: profileId,
    };
    return catalog;
  }),
};

vi.mock('../api', () => apiMock);

const { default: ProfileCatalogSection } = await import('../components/settings/ProfileCatalogSection.svelte');

function buildCatalog(): ProfileCatalogResponse {
  return {
    mode: 'settings_profiles_v2',
    manifest_present: true,
    default_profile: 'everyday-work',
    catalog_source: 'file:///profiles/profile-manifest.json',
    profiles: [
      {
        profile_id: 'everyday-work',
        current_revision: '2026.0520.2',
        installed_revision: '2026.0520.1',
        revisions: [
          {
            revision: '2026.0520.1',
            status: 'deprecated',
            min_binary: '1.0.0',
            profile_hash: 'blake3:eeee',
            current: false,
            installed: true,
          },
          {
            revision: '2026.0520.2',
            status: 'active',
            min_binary: '1.0.0',
            profile_hash: 'blake3:ffff',
            current: true,
            installed: false,
          },
        ],
      },
      {
        profile_id: 'locked-corp',
        current_revision: '2026.0520.1',
        installed_revision: null,
        revisions: [
          {
            revision: '2026.0520.1',
            status: 'revoked',
            min_binary: '1.0.0',
            profile_hash: 'blake3:cccc',
            current: true,
            installed: false,
          },
        ],
      },
    ],
  };
}

describe('ProfileCatalogSection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    catalog = buildCatalog();
  });

  it('renders profile catalog revision lifecycle states without removed status', async () => {
    render(ProfileCatalogSection);

    await screen.findByText('everyday-work');

    expect(screen.getByText('locked-corp')).toBeTruthy();
    expect(screen.getByText('selected')).toBeTruthy();
    expect(screen.getByText('update available')).toBeTruthy();
    expect(screen.getByText('not installed')).toBeTruthy();
    expect(screen.getAllByText('2026.0520.1').length).toBeGreaterThan(0);
    expect(screen.getAllByText('2026.0520.2').length).toBeGreaterThan(0);
    expect(screen.getByText('deprecated')).toBeTruthy();
    expect(screen.getByText('active')).toBeTruthy();
    expect(screen.getByText('revoked')).toBeTruthy();
    expect(screen.queryByText('removed')).toBeNull();
  });

  it('selects a non-revoked profile through the profile route', async () => {
    catalog = buildCatalog();
    catalog.profiles[1].revisions[0].status = 'active';
    render(ProfileCatalogSection);

    await screen.findByText('locked-corp');
    const buttons = screen.getAllByRole('button', { name: 'Select' });
    await fireEvent.click(buttons[0]);

    expect(apiMock.selectProfile).toHaveBeenCalledWith('locked-corp');
    await waitFor(() => {
      expect(screen.getByText('locked-corp selected.')).toBeTruthy();
    });
  });

  it('does not allow revoked profiles to be selected', async () => {
    render(ProfileCatalogSection);

    await screen.findByText('locked-corp');
    const selectButtons = screen.getAllByRole<HTMLButtonElement>('button', { name: 'Select' });
    expect(selectButtons[0].disabled).toBe(true);
    expect(apiMock.selectProfile).not.toHaveBeenCalled();
  });

  it('refreshes the catalog on demand', async () => {
    render(ProfileCatalogSection);

    await screen.findByText('everyday-work');
    catalog = {
      mode: 'settings_profiles_v2',
      manifest_present: true,
      default_profile: null,
      profiles: [],
    };

    await fireEvent.click(screen.getByRole('button', { name: 'Refresh profiles' }));

    await waitFor(() => {
      expect(screen.getByText('No profile catalog installed.')).toBeTruthy();
    });
    expect(apiMock.getProfileCatalog).toHaveBeenCalledTimes(2);
  });
});
