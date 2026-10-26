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
export function composition(rows: Snapshot[]): Variant[] {
  return rows.map(row => {
    const sum = total(row.loc);
    return { label: row.date, spec: {
      id: 'loc-pie', title: 'Share of source code', type: 'pie', unit: '%',
      labels: Object.values(components), series: [{ name: 'Share of total LOC', values: Object.keys(components).map(key => 100 * (row.loc[key] ?? 0) / sum) }],
      note: `${sum.toLocaleString('en-US')} source lines · ${row.date}. All categories use the same percentage scale.`,
    } };
  });
}
export function clientComposition(rows: Snapshot[]): Variant[] {
  const names: Record<string, string> = { app: 'App', cli: 'CLI', tui: 'TUI', tray: 'Tray', mcp_host: 'MCP', sdk_rust: 'Rust SDK', sdk_python: 'Python SDK', sdk_typescript: 'TypeScript SDK' };
  const keys = [...new Set([...Object.keys(names), ...rows.flatMap(r => Object.keys(r.clientLoc))])];
  const maximum = Math.max(1, ...rows.flatMap(r => Object.values(r.clientLoc)));
  const radarMax = Math.ceil(maximum / 1000) * 1000;
  return rows.map(row => ({ label: row.date, spec: {
    id: 'client-breakdown', title: 'Client code breakdown', type: 'radar', unit: 'LOC', radarMax,
    labels: keys.map(key => names[key] ?? key.replaceAll('_', ' ')),
    series: [{ name: 'Source lines', values: keys.map(key => row.clientLoc[key] ?? 0) }],
    note: `Client source lines at ${row.date}. Every spoke uses the same 0–${radarMax.toLocaleString('en-US')} LOC scale across all snapshots; generated code and tests excluded.`,
  } }));
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
    releases: { id: 'release-success-rate', title: 'Successful release attempts over time', type: 'line', unit: '%',
      labels: data.releaseOutcomes.map(r => r.month), series: [{ name: 'Successful attempts', values: data.releaseOutcomes.map(r => {
        const attempts = r.success + r.failure + r.cancelled;
        return attempts ? 100 * r.success / attempts : null;
      }) }],
      note: 'Monthly success rate = successful attempts ÷ (successful + failed + cancelled attempts). Each rerun counts separately; binary and runtime workflows are included. Months with no attempts have no value.',
    },
  };
}
