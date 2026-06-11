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
