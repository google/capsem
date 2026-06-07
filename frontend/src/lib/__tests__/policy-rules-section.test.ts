// @vitest-environment jsdom

import { describe, it, expect, beforeEach } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/svelte';
import PolicyRulesSection from '../components/settings/PolicyRulesSection.svelte';
import { SettingsModel } from '../models/settings-model';
import { settingsStore } from '../stores/settings.svelte';
import { buildMockSettingsResponse } from '../mock-settings';
import type { SettingsNode, SettingsResponse } from '../types/settings';

function renderPolicy(response: SettingsResponse = buildMockSettingsResponse()) {
  settingsStore.model = new SettingsModel(response);
  settingsStore.loading = false;
  settingsStore.error = null;
  settingsStore.reloadError = null;
  return render(PolicyRulesSection);
}

function setLeafValue(nodes: SettingsNode[], id: string, value: unknown): boolean {
  for (const node of nodes) {
    if (node.kind === 'leaf' && node.id === id) {
      (node as { effective_value: unknown }).effective_value = value;
      return true;
    }
    if (node.kind === 'group' && setLeafValue(node.children, id, value)) {
      return true;
    }
  }
  return false;
}

describe('PolicyRulesSection', () => {
  beforeEach(() => {
    settingsStore.model = null;
  });

  it('hides unsupported hook and dns.response controls', async () => {
    renderPolicy();

    expect(screen.queryByRole('button', { name: 'hook' })).toBeNull();
    expect(screen.queryByText('hook.decision')).toBeNull();

    await fireEvent.click(screen.getByRole('button', { name: 'dns' }));
    expect(screen.queryByText('dns.response')).toBeNull();
  });

  it('renders a staged add before save', async () => {
    renderPolicy();

    await fireEvent.input(screen.getByPlaceholderText('block_prod_token'), {
      target: { value: 'block_evil' },
    });
    await fireEvent.click(screen.getByRole('button', { name: /stage rule/i }));

    expect(settingsStore.model!.pendingChanges.get('policy.http.block_evil')).toMatchObject({
      on: 'http.request',
      decision: 'block',
    });
    expect(screen.getByText('staged add')).toBeTruthy();
    expect(screen.getByText('block_evil')).toBeTruthy();
  });

  it('stages rename as old-key delete plus new-key add', async () => {
    renderPolicy();

    await fireEvent.click(screen.getByText('block_openai_github'));
    await fireEvent.input(screen.getByPlaceholderText('block_prod_token'), {
      target: { value: 'block_github_org' },
    });
    await fireEvent.click(screen.getByRole('button', { name: /stage rule/i }));

    expect(settingsStore.model!.pendingChanges.get('policy.http.block_openai_github')).toBeNull();
    expect(settingsStore.model!.pendingChanges.get('policy.http.block_github_org')).toMatchObject({
      on: 'http.request',
      decision: 'block',
    });
    expect(screen.getByText('staged add')).toBeTruthy();
    expect(screen.getByText('delete')).toBeTruthy();
  });

  it('stages type change as old-key delete plus new typed key', async () => {
    renderPolicy();

    await fireEvent.click(screen.getByRole('button', { name: 'mcp' }));
    await fireEvent.click(screen.getByText('ask_prod_issue'));
    await fireEvent.change(screen.getByLabelText('Type'), { target: { value: 'http' } });
    await fireEvent.input(screen.getByPlaceholderText('block_prod_token'), {
      target: { value: 'block_prod_http' },
    });
    await fireEvent.input(screen.getByPlaceholderText('request.host == "github.com"'), {
      target: { value: 'request.host == "prod.example.com"' },
    });
    await fireEvent.click(screen.getByRole('button', { name: /stage rule/i }));

    expect(settingsStore.model!.pendingChanges.get('policy.mcp.ask_prod_issue')).toBeNull();
    expect(settingsStore.model!.pendingChanges.get('policy.http.block_prod_http')).toMatchObject({
      on: 'http.request',
      decision: 'ask',
    });
  });

  it('renders staged deletes before save', async () => {
    renderPolicy();

    await fireEvent.click(screen.getAllByTitle('Delete rule')[0]);
    expect(settingsStore.model!.pendingChanges.get('policy.http.block_openai_github')).toBeNull();
    expect(screen.getByText('delete')).toBeTruthy();
  });

  it('stages generated candidates from settings chips', async () => {
    const response = buildMockSettingsResponse();
    expect(setLeafValue(response.tree, 'security.web.custom_block', 'evil.com')).toBe(true);
    renderPolicy(response);

    await fireEvent.click(screen.getByRole('button', { name: /stage all/i }));
    expect(settingsStore.model!.pendingChanges.get('policy.http.block_custom_evil_com')).toMatchObject({
      on: 'http.request',
      decision: 'block',
    });
  });
});
