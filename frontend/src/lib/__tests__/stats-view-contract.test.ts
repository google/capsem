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
    expect(source).toContain('Captured');
    expect(source).toContain('Brokered');
    expect(source).toContain('Injected');
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

  it('shows credential broker verbs instead of reference hashes or status columns', () => {
    const credentialsStart = source.indexOf("{:else if activeTab === 'credentials'}");
    const securityStart = source.indexOf("{:else if activeTab === 'security'}");
    expect(credentialsStart).toBeGreaterThan(-1);
    expect(securityStart).toBeGreaterThan(credentialsStart);

    const credentialsBlock = source.slice(credentialsStart, securityStart);
    expect(credentialsBlock).toContain('brokerVerb(row)');
    expect(credentialsBlock).toContain("columns={['Time', 'Verb', 'Source', 'Provider', 'Origin']}");
    expect(credentialsBlock).toContain('Captured');
    expect(credentialsBlock).toContain('Brokered');
    expect(credentialsBlock).toContain('Injected');
    expect(credentialsBlock).not.toContain('Substituted');
    expect(credentialsBlock).not.toContain('References');
    expect(credentialsBlock).not.toContain('Outcome');
    expect(credentialsBlock).not.toContain('substitution_ref');

    expect(source).toContain("'substitution_ref'");
    expect(source).toContain("'credential_ref'");
  });
});

describe('StatsView detail drawer contract', () => {
  it('does not render the selected event twice as raw JSON plus repeated fields', () => {
    expect(source).not.toContain("formatAndHighlight(detail.data, 'json')");
    expect(source).toContain('visibleDetailEntries(detail.data)');
    expect(source).toContain('detailPayloadSections(detail.data)');
  });
});

describe('StatsView file summary contract', () => {
  it('summarizes file actions visible in the event table', () => {
    const filesStart = source.indexOf("{:else if activeTab === 'files'}");
    const processStart = source.indexOf("{:else if activeTab === 'process'}");
    expect(filesStart).toBeGreaterThan(-1);
    expect(processStart).toBeGreaterThan(filesStart);

    const filesBlock = source.slice(filesStart, processStart);
    expect(filesBlock).toContain('Created');
    expect(filesBlock).toContain('Modified');
    expect(filesBlock).toContain('Deleted');
    expect(filesBlock).not.toContain('Imports');
    expect(filesBlock).not.toContain('Exports');
    expect(filesBlock).not.toContain('Brokered Refs');
  });
});
