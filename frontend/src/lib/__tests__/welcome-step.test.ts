// @vitest-environment jsdom

import { render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';

const { default: WelcomeStep } = await import('../components/onboarding/WelcomeStep.svelte');

describe('WelcomeStep', () => {
  it('renders a durable welcome without release notes or asset status', () => {
    render(WelcomeStep);

    expect(screen.getByRole('heading', { name: 'Welcome to Capsem' })).toBeTruthy();
    expect(screen.queryByText("What's New")).toBeNull();
    expect(screen.queryByText('VM Assets')).toBeNull();
    expect(screen.queryByText('Refresh status')).toBeNull();
  });
});
