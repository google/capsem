// @vitest-environment jsdom

import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { DebugReport } from '../types/gateway';

let debugReport: DebugReport;

const apiMock = {
  getDebugReport: vi.fn(async () => debugReport),
};

vi.mock('../api', () => apiMock);

const { default: SecurityEngineHealthSection } = await import('../components/settings/SecurityEngineHealthSection.svelte');

function buildDebugReport(): DebugReport {
  return {
    text: 'Capsem Debug Report',
    json: {
      schema: 'capsem.debug.v2',
      redacted: true,
      security_engine: {
        present: true,
        runtime_rules_store_enabled: true,
        runtime_rules_store_path: '/tmp/capsem/runtime-security-rules.json',
        enforcement: {
          rule_count: 3,
          enabled_count: 2,
          compiled_count: 2,
          error_count: 1,
          runtime_scope_count: 1,
          profile_scope_count: 2,
          scope_counts: { profile: 2, runtime: 1 },
          match_count_total: 9,
          latest_match_unix_ms: 1700000000000,
          rules: [],
        },
        detection: {
          rule_count: 4,
          enabled_count: 4,
          compiled_count: 4,
          error_count: 0,
          runtime_scope_count: 1,
          profile_scope_count: 3,
          scope_counts: { profile: 3, runtime: 1 },
          match_count_total: 12,
          latest_match_unix_ms: 1700000001000,
          rules: [],
        },
        confirm: {
          resolver_available: false,
          owner: 'service',
        },
      },
    },
  };
}

describe('SecurityEngineHealthSection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    debugReport = buildDebugReport();
  });

  it('renders typed security engine health from the debug report', async () => {
    render(SecurityEngineHealthSection);

    await screen.findByText('Security Engine Health');

    expect(screen.getByText('Enforcement')).toBeTruthy();
    expect(screen.getByText('Detection')).toBeTruthy();
    expect(screen.getAllByText('3').length).toBeGreaterThan(0);
    expect(screen.getAllByText('4').length).toBeGreaterThan(0);
    expect(screen.getByText('1 compile error')).toBeTruthy();
    expect(screen.getByText('4/4 compiled')).toBeTruthy();
    expect(screen.getByText('9')).toBeTruthy();
    expect(screen.getByText('12')).toBeTruthy();
    expect(screen.getByText('enabled')).toBeTruthy();
    expect(screen.getByText('unavailable')).toBeTruthy();
    expect(screen.getByText('service')).toBeTruthy();
    expect(screen.getByText('/tmp/capsem/runtime-security-rules.json')).toBeTruthy();
  });

  it('refreshes health on demand', async () => {
    render(SecurityEngineHealthSection);

    await screen.findByText('1 compile error');
    debugReport = buildDebugReport();
    debugReport.json.security_engine.enforcement.error_count = 0;
    debugReport.json.security_engine.enforcement.compiled_count = 3;

    await fireEvent.click(screen.getByRole('button', { name: 'Refresh security health' }));

    await waitFor(() => {
      expect(screen.getByText('3/3 compiled')).toBeTruthy();
    });
    expect(apiMock.getDebugReport).toHaveBeenCalledTimes(2);
  });

  it('fails closed when the debug report has no security engine block', async () => {
    debugReport = {
      text: 'Capsem Debug Report',
      json: undefined,
    };

    render(SecurityEngineHealthSection);

    await screen.findByText('Security engine health is unavailable in the debug report.');
  });
});
