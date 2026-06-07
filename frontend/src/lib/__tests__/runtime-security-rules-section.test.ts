// @vitest-environment jsdom

import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { RuntimeRuleEntry } from '../types/gateway';

const enforcementRows: RuntimeRuleEntry[] = [
  {
    id: 'profile-block-admin',
    pack_id: 'default-profile',
    scope: 'profile',
    origin: 'profile',
    priority: 10,
    definition: {
      kind: 'enforcement',
      decision: 'block',
      reason: 'Profile rule',
    },
    enabled: true,
    compiled: true,
    compile_status: { status: 'compiled' },
    generation: 2,
    condition: "http.request.path.startsWith('/admin')",
    compiled_plan: 'cel:profile',
    match_count: 7,
    last_matched_event: 'evt-admin',
    last_matched_unix_ms: 1700000000000,
  },
  {
    id: 'runtime-ask-token',
    pack_id: 'runtime',
    scope: 'runtime',
    origin: 'runtime',
    priority: 80,
    definition: {
      kind: 'enforcement',
      decision: 'ask',
      reason: 'Token egress',
    },
    enabled: true,
    compiled: true,
    compile_status: { status: 'compiled' },
    generation: 1,
    condition: "http.request.header('authorization').exists()",
    compiled_plan: 'cel:runtime',
    match_count: 3,
    last_matched_event: null,
    last_matched_unix_ms: null,
  },
];

const detectionRows: RuntimeRuleEntry[] = [
  {
    id: 'detect-secret-egress',
    pack_id: 'runtime-detection',
    scope: 'runtime',
    origin: 'runtime',
    priority: 60,
    definition: {
      kind: 'detection',
      sigma_id: 'capsem-secret-egress',
      title: 'Secret egress',
      severity: 'high',
      confidence: 'high',
      tags: ['http', 'egress'],
    },
    enabled: true,
    compiled: true,
    compile_status: { status: 'compiled' },
    generation: 4,
    condition: "http.request.body.text.contains('secret')",
    compiled_plan: 'cel:detection',
    match_count: 11,
    last_matched_event: 'evt-secret',
    last_matched_unix_ms: 1700000001000,
  },
];

const apiMock = {
  getRuntimeEnforcementRules: vi.fn(async () => ({ kind: 'enforcement', rules: enforcementRows })),
  getRuntimeDetectionRules: vi.fn(async () => ({ kind: 'detection', rules: detectionRows })),
  validateRuntimeEnforcementRule: vi.fn(async () => ({
    compiled: true,
    id: 'runtime-block-google',
    compiled_plan: 'cel:google',
  })),
  validateRuntimeDetectionRule: vi.fn(async () => ({
    compiled: true,
    id: 'runtime-detect-google',
    compiled_plan: 'cel:detect-google',
  })),
  installRuntimeEnforcementRule: vi.fn(async () => ({
    kind: 'enforcement',
    rule: enforcementRows[1],
  })),
  installRuntimeDetectionRule: vi.fn(async () => ({
    kind: 'detection',
    rule: detectionRows[0],
  })),
  backtestRuntimeEnforcementRule: vi.fn(async () => ({
    total_matches: 1,
    unique_evidence_matches: 1,
    truncated: false,
    rows: [
      {
        event_ref: { event_id: 'sample-http-request' },
        rule_id: 'runtime-block-google',
        pack_id: 'runtime',
        evidence_signature: 'http.request.host=google.com',
        matched_fields: [{ path: 'http.request.host', value: 'google.com' }],
        outcome: { action: 'block' },
      },
    ],
  })),
  backtestRuntimeDetectionRule: vi.fn(async () => ({
    total_matches: 1,
    unique_evidence_matches: 1,
    truncated: false,
    rows: [
      {
        event_ref: { event_id: 'sample-http-request' },
        rule_id: 'runtime-detect-google',
        pack_id: 'runtime-detection',
        evidence_signature: 'http.request.body.text=secret',
        matched_fields: [{ path: 'http.request.body.text', value: 'secret token' }],
        outcome: { severity: 'high' },
      },
    ],
  })),
  huntSessionRuntimeDetectionRules: vi.fn(async () => ({
    total_matches: 1,
    unique_evidence_matches: 1,
    truncated: false,
    rows: [
      {
        event_ref: { event_id: 'evt-secret', session_id: 'vm 1' },
        rule_id: 'runtime-detect-google',
        pack_id: 'runtime-detection',
        evidence_signature: 'session:http.request.body.text=secret',
        matched_fields: [{ path: 'http.request.body.text', value: 'secret token' }],
        outcome: { severity: 'high' },
      },
    ],
  })),
  deleteRuntimeEnforcementRule: vi.fn(async () => ({
    kind: 'enforcement',
    id: 'runtime-ask-token',
    removed: true,
  })),
  deleteRuntimeDetectionRule: vi.fn(async () => ({
    kind: 'detection',
    id: 'detect-secret-egress',
    removed: true,
  })),
};

vi.mock('../api', () => apiMock);

const { default: RuntimeSecurityRulesSection } = await import('../components/settings/RuntimeSecurityRulesSection.svelte');

describe('RuntimeSecurityRulesSection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('loads enforcement and detection runtime rules with priority and attribution', async () => {
    render(RuntimeSecurityRulesSection);

    await screen.findByText('profile-block-admin');
    expect(screen.getByText('priority 10')).toBeTruthy();
    expect(screen.getAllByText('profile')).toHaveLength(2);
    expect(screen.getByText('7 matches')).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: 'Detection' }));

    await screen.findByText('detect-secret-egress');
    expect(screen.getByText('priority 60')).toBeTruthy();
    expect(screen.getByText('11 matches')).toBeTruthy();
    expect(screen.getByText('Secret egress')).toBeTruthy();
  });

  it('validates and installs enforcement drafts with priority', async () => {
    render(RuntimeSecurityRulesSection);

    await screen.findByText('profile-block-admin');
    await fireEvent.input(screen.getByLabelText('Rule id'), {
      target: { value: 'runtime-block-google' },
    });
    await fireEvent.input(screen.getByLabelText('Pack id'), {
      target: { value: 'runtime' },
    });
    await fireEvent.input(screen.getByLabelText('Priority'), {
      target: { value: '55' },
    });
    await fireEvent.input(screen.getByLabelText('Condition'), {
      target: { value: "http.request.host.contains('google')" },
    });
    await fireEvent.change(screen.getByLabelText('Decision'), {
      target: { value: 'block' },
    });
    await fireEvent.input(screen.getByLabelText('Reason'), {
      target: { value: 'No Google egress' },
    });

    await fireEvent.click(screen.getByRole('button', { name: /validate/i }));

    expect(apiMock.validateRuntimeEnforcementRule).toHaveBeenCalledWith({
      id: 'runtime-block-google',
      pack_id: 'runtime',
      priority: 55,
      condition: "http.request.host.contains('google')",
      decision: 'block',
      reason: 'No Google egress',
      enabled: true,
    });

    await fireEvent.click(screen.getByRole('button', { name: /install/i }));

    expect(apiMock.installRuntimeEnforcementRule).toHaveBeenCalledWith({
      id: 'runtime-block-google',
      pack_id: 'runtime',
      priority: 55,
      condition: "http.request.host.contains('google')",
      decision: 'block',
      reason: 'No Google egress',
      enabled: true,
    });
    expect(apiMock.getRuntimeEnforcementRules).toHaveBeenCalledTimes(2);
    expect(apiMock.getRuntimeDetectionRules).toHaveBeenCalledTimes(2);
  });

  it('backtests enforcement drafts against a JSON event corpus', async () => {
    render(RuntimeSecurityRulesSection);

    await screen.findByText('profile-block-admin');
    await fireEvent.input(screen.getByLabelText('Rule id'), {
      target: { value: 'runtime-block-google' },
    });
    await fireEvent.input(screen.getByLabelText('Condition'), {
      target: { value: "http.request.host.contains('google')" },
    });

    await fireEvent.click(screen.getByRole('button', { name: /backtest/i }));

    expect(apiMock.backtestRuntimeEnforcementRule).toHaveBeenCalledWith({
      rule: {
        id: 'runtime-block-google',
        pack_id: 'runtime',
        priority: 100,
        condition: "http.request.host.contains('google')",
        decision: 'block',
        reason: null,
        enabled: true,
      },
      events: [
        {
          event_ref: { event_id: 'sample-http-request' },
          event: {
            event_family: 'http',
            event_type: 'http.request',
            subject: {
              host: 'google.com',
              path: '/admin',
              body: { text: 'secret token' },
            },
          },
        },
      ],
      limit: 100,
    });
    expect(await screen.findByText('http.request.host=google.com')).toBeTruthy();
    expect(screen.getByText('http.request.host')).toBeTruthy();
  });

  it('hunts a session with a draft detection rule', async () => {
    render(RuntimeSecurityRulesSection);

    await screen.findByText('profile-block-admin');
    await fireEvent.click(screen.getByRole('button', { name: 'Detection' }));
    await fireEvent.input(screen.getByLabelText('Rule id'), {
      target: { value: 'runtime-detect-google' },
    });
    await fireEvent.input(screen.getByLabelText('Condition'), {
      target: { value: "http.request.body.text.contains('secret')" },
    });
    await fireEvent.input(screen.getByLabelText('Title'), {
      target: { value: 'Secret egress' },
    });
    await fireEvent.input(screen.getByLabelText('Session id'), {
      target: { value: 'vm 1' },
    });

    await fireEvent.click(screen.getByRole('button', { name: /hunt session/i }));

    expect(apiMock.huntSessionRuntimeDetectionRules).toHaveBeenCalledWith('vm 1', {
      rules: [
        {
          id: 'runtime-detect-google',
          pack_id: 'runtime',
          sigma_id: null,
          title: 'Secret egress',
          priority: 100,
          condition: "http.request.body.text.contains('secret')",
          severity: 'medium',
          confidence: 'high',
          tags: [],
          enabled: true,
        },
      ],
      limit: 100,
    });
    expect(await screen.findByText('session:http.request.body.text=secret')).toBeTruthy();
  });

  it('protects profile-owned rows and deletes runtime overlays', async () => {
    render(RuntimeSecurityRulesSection);

    const profileRow = (await screen.findByText('profile-block-admin')).closest('article');
    expect(profileRow).not.toBeNull();
    expect(within(profileRow!).getByRole<HTMLButtonElement>('button', { name: /delete/i }).disabled).toBe(true);

    const runtimeRow = screen.getByText('runtime-ask-token').closest('article');
    expect(runtimeRow).not.toBeNull();
    await fireEvent.click(within(runtimeRow!).getByRole('button', { name: /delete/i }));

    expect(apiMock.deleteRuntimeEnforcementRule).toHaveBeenCalledWith('runtime-ask-token');
    await waitFor(() => {
      expect(apiMock.getRuntimeEnforcementRules).toHaveBeenCalledTimes(2);
    });
  });
});
