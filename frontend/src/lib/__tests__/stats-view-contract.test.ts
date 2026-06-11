import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const source = readFileSync(
  new URL('../components/views/StatsView.svelte', import.meta.url),
  'utf8',
);

describe('StatsView process contract', () => {
  it('distinguishes command executions from process observations', () => {
    expect(source).toContain('Process Exec Events');
    expect(source).toContain('Process Observations');
    expect(source).toContain('audit-port process records');
    expect(source).toContain("type: 'process observation'");
    expect(source).not.toContain('Process Audit Events');
    expect(source).not.toContain("type: 'process audit'");
  });
});

describe('StatsView snapshot boundary', () => {
  it('does not expose hypervisor snapshots as a generic stats tab', () => {
    expect(source).not.toContain("id: 'snapshots'");
    expect(source).not.toContain('snapshot_events');
    expect(source).not.toContain('Snapshot Events');
    expect(source).toContain("id: 'mcp'");
  });
});

describe('StatsView credential broker contract', () => {
  it('surfaces broker evidence as a first-class tab instead of process activity', () => {
    expect(source).toContain("'credentials'");
    expect(source).toContain("label: 'Credentials'");
    expect(source).toContain('Credential Broker Events');
    expect(source).toContain("type: 'credential broker event'");
    expect(source).toContain('substitution_events');
    expect(source).toContain('Substituted');
    expect(source).not.toContain('Credential Substitutions');

    const processStart = source.indexOf("{:else if activeTab === 'process'}");
    const credentialsStart = source.indexOf("{:else if activeTab === 'credentials'}");
    const securityStart = source.indexOf("{:else if activeTab === 'security'}");
    expect(processStart).toBeGreaterThan(-1);
    expect(credentialsStart).toBeGreaterThan(processStart);
    expect(securityStart).toBeGreaterThan(credentialsStart);

    const processBlock = source.slice(processStart, credentialsStart);
    expect(processBlock).not.toContain('substitutionRows');
    expect(processBlock).not.toContain('Credential Broker Events');
  });
});
