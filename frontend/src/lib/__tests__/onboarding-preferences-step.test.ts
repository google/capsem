// @vitest-environment jsdom

import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProfileCatalogResponse } from '../types/gateway';

let catalog: ProfileCatalogResponse;

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
  getProfileCatalog: vi.fn(async () => catalog),
  selectProfile: vi.fn(async (profileId: string) => {
    catalog = { ...catalog, default_profile: profileId };
    return catalog;
  }),
  getSettings: vi.fn(async () => ({ tree: [], issues: [], presets: [] })),
  saveSettings: vi.fn(async () => ({ tree: [], issues: [], presets: [] })),
};

vi.mock('../api', () => apiMock);

const { default: PreferencesStep } = await import('../components/onboarding/PreferencesStep.svelte');

function buildCatalog(): ProfileCatalogResponse {
  return {
    mode: 'settings_profiles_v2',
    manifest_present: true,
    default_profile: 'coding',
    profiles: [
      {
        profile_id: 'coding',
        current_revision: '2026.0520.1',
        installed_revision: '2026.0520.1',
        revisions: [
          {
            revision: '2026.0520.1',
            status: 'active',
            current: true,
            installed: true,
          },
        ],
      },
      {
        profile_id: 'everyday-work',
        current_revision: '2026.0520.1',
        installed_revision: '2026.0520.1',
        revisions: [
          {
            revision: '2026.0520.1',
            status: 'active',
            current: true,
            installed: true,
          },
        ],
      },
      {
        profile_id: 'revoked-profile',
        current_revision: '2026.0520.1',
        installed_revision: '2026.0520.1',
        revisions: [
          {
            revision: '2026.0520.1',
            status: 'revoked',
            current: true,
            installed: true,
          },
        ],
      },
    ],
  };
}

describe('PreferencesStep', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    catalog = buildCatalog();
  });

  it('selects onboarding profiles through the Profile V2 catalog route', async () => {
    render(PreferencesStep);

    await screen.findByText('Profile');
    const profileSelect = screen.getAllByRole<HTMLSelectElement>('combobox')[0];
    expect(profileSelect.value).toBe('coding');
    expect(screen.getByText('Profile')).toBeTruthy();
    expect(screen.queryByText('Security Preset')).toBeNull();

    await fireEvent.change(profileSelect, { target: { value: 'everyday-work' } });

    await waitFor(() => {
      expect(apiMock.selectProfile).toHaveBeenCalledWith('everyday-work');
    });
  });

  it('does not offer revoked profiles as selectable wizard choices', async () => {
    render(PreferencesStep);

    await screen.findByText('Profile');
    const option = screen.getByRole<HTMLOptionElement>('option', {
      name: 'revoked-profile@2026.0520.1',
    });
    expect(option.disabled).toBe(true);
  });
});
