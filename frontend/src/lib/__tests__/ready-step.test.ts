// @vitest-environment jsdom

import { render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { apiMock, state } = vi.hoisted(() => {
  const state = {
    listProfilesFails: false,
  };
  const apiMock = {
    listProfiles: vi.fn(async () => {
      if (state.listProfilesFails) throw new Error('service offline');
      return {
        mode: 'settings_profiles_v2',
        default_profile: 'coding',
        profiles: [
          {
            source: 'base',
            locked: false,
            profile: {
              id: 'coding',
              name: 'Coding',
              description: 'Focused defaults for software development sessions.',
              best_for: 'Coding agents, repository work, tests, and developer tooling.',
              ui: 'coding',
            },
          },
          {
            source: 'base',
            locked: false,
            profile: {
              id: 'everyday-work',
              name: 'Everyday Work',
              description: 'Balanced defaults for daily work sessions.',
              best_for: 'Daily work with useful tools and measured security prompts.',
              ui: 'everyday',
            },
          },
        ],
      };
    }),
  };
  return { apiMock, state };
});

vi.mock('../api', () => apiMock);

const { default: ReadyStep } = await import('../components/onboarding/ReadyStep.svelte');

describe('ReadyStep', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    state.listProfilesFails = false;
  });

  it('introduces sessions and profiles without readiness jargon', async () => {
    render(ReadyStep);

    await screen.findByText("You're ready to start");
    expect(screen.getByText(/Start a session with the profile/)).toBeTruthy();
    expect(screen.getByText('Coding')).toBeTruthy();
    expect(screen.getByText('Everyday Work')).toBeTruthy();
    expect(screen.getByText('Default')).toBeTruthy();
    expect(screen.getByText(/New Session/)).toBeTruthy();
    expect(screen.queryByText('VM Assets')).toBeNull();
    expect(screen.queryByText('Service offline')).toBeNull();
    expect(screen.queryByText(/readiness/i)).toBeNull();
  });

  it('falls back to built-in profile cards when the service is unavailable', async () => {
    state.listProfilesFails = true;
    render(ReadyStep);

    await waitFor(() => {
      expect(apiMock.listProfiles).toHaveBeenCalled();
    });
    expect(screen.getByText('Coding')).toBeTruthy();
    expect(screen.getByText('Everyday Work')).toBeTruthy();
    expect(screen.queryByText('Service offline')).toBeNull();
  });
});
