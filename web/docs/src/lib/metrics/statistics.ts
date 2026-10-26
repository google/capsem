import type { ChartSpec, Snapshot, Statistics, Variant } from './types';

export const components: Record<string, string> = {
  hypervisor: 'Hypervisor', clients: 'Clients', vm_guest: 'Guest agent & scripts', vm_host: 'Host VM lifecycle',
  host_other: 'Other host components', tests: 'Tests & support', citadel_guardrails: 'Citadel & guardrails',
  benchmarks: 'Benchmark code', build_release_tooling: 'Build & release tooling', docs_marketing: 'Docs & marketing', other: 'Other code',
};
export const testSystems: Record<string, string> = {
  rust: 'Rust', python_build_system: 'Python build tooling', python_vm_integration: 'VM integration',
  frontend: 'Frontend', citadel: 'Citadel', legacy_source_contracts: 'Source contracts',
  doctor: 'Doctor', ironbank: 'Ironbank', python_sdk: 'Python SDK', typescript_sdk: 'TypeScript SDK',
  mcp_typescript: 'MCP', inspect_ai: 'Inspect AI', python_other: 'Other Python', javascript_other: 'Other JavaScript',
};
export function total(values: Record<string, number>): number { return Object.values(values).reduce((a, b) => a + b, 0); }
export function composition(rows: Snapshot[], type: 'pie' | 'radar'): Variant[] {
  return rows.map(row => {
    const sum = total(row.loc);
    return { label: row.date, spec: {
      id: `loc-${type}`, title: type === 'pie' ? 'Share of source code' : 'Code composition radar', type, unit: '%',
      labels: Object.values(components), series: [{ name: 'Share of total LOC', values: Object.keys(components).map(key => 100 * (row.loc[key] ?? 0) / sum) }],
      note: `${sum.toLocaleString('en-US')} source lines · ${row.date}. All categories use the same percentage scale.`,
    } };
  });
}
export function statisticCharts(data: Statistics): Record<string, ChartSpec> {
  const rows = data.snapshots, labels = rows.map(r => r.date);
  const line = (id: string, title: string, unit: string, series: ChartSpec['series'], stacked = false): ChartSpec => ({ id, title, type: 'line', unit, labels, series, stacked });
  return {
    code: line('code-growth', 'Code growth from the first commit', 'LOC', Object.entries(components).map(([key, name]) => ({ name, values: rows.map(r => r.loc[key] ?? 0) })), true),
    crates: line('crate-growth', 'Crates and internal connections', 'count', [
      { name: 'Workspace crates', values: rows.map(r => r.crates.length) },
      { name: 'Internal dependency pairs', values: rows.map(r => new Set(r.edges.map(e => `${e.from}/${e.to}`)).size) },
    ]),
    dependencies: line('external-dependencies', 'Direct external dependencies', 'packages', ['rust', 'npm', 'python'].map(key => ({ name: key, values: rows.map(r => r.dependencies[key].direct) }))),
    tests: line('test-growth', 'Declared correctness tests by system', 'definitions', Object.entries(testSystems).map(([key, name]) => ({ name, values: rows.map(r => r.tests[key] ?? 0) })), true),
    releases: { id: 'release-outcomes', title: 'Recorded release workflow outcomes', type: 'bar', unit: 'attempts', stacked: true,
      labels: data.releaseOutcomes.map(r => r.month), series: ['success', 'failure', 'cancelled'].map(key => ({ name: key, values: data.releaseOutcomes.map(r => r[key as 'success' | 'failure' | 'cancelled']) })) },
    failureIntervals: { id: 'release-failure-intervals', title: 'Failures between successful releases', type: 'bar', unit: 'attempts',
      labels: data.releaseSuccesses.map(r => r.label), series: [{ name: 'Failures since prior success', values: data.releaseSuccesses.map(r => r.failures) }],
      note: 'Counted independently for binary and runtime lanes, by completion time. The first success includes all failures since the start of retained evidence. Cancellations are excluded.' },
    successTime: { id: 'successful-release-time', title: 'Duration of each successful release workflow', type: 'bar', unit: 'minutes',
      labels: data.releaseSuccesses.map(r => r.label), series: [{ name: 'Wall-clock duration', values: data.releaseSuccesses.map(r => r.minutes) }] },
    releaseTime: { id: 'release-duration', title: 'Workflow duration for published binary releases', type: 'bar', unit: 'minutes',
      labels: data.releasePublications.map(r => r.version), series: [{ name: 'Producing workflow', values: data.releasePublications.map(r => r.minutes) }],
      note: 'Publication and workflow success are different milestones. A producing workflow can finish with a failure after publishing artifacts.' },
  };
}
