// @vitest-environment jsdom

import { describe, it, expect, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';

vi.mock('../api', () => ({
  getSettings: vi.fn(async () => {
    throw new Error('settings unavailable');
  }),
  runDetection: vi.fn(async () => ({})),
  saveSettings: vi.fn(async () => ({})),
  validateApiKey: vi.fn(async () => ({ valid: true, message: 'ok' })),
}));

const { default: ProvidersStep } = await import('../components/onboarding/ProvidersStep.svelte');

describe('ProvidersStep', () => {
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
});
