import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { dependencyView } from '../src/lib/metrics/dependencies.ts';
import { clientComposition, composition, statisticCharts, total } from '../src/lib/metrics/statistics.ts';

const data = JSON.parse(readFileSync(new URL('../public/data/metrics/statistics.json', import.meta.url), 'utf8'));
const latest = data.snapshots.at(-1);

test('monthly history begins at the actual original commit and keeps snapshot identity', () => {
  assert.equal(data.snapshots[0].date, '2025-10-01');
  assert.equal(data.snapshots[0].crates.length, 0);
  assert.equal(new Set(data.snapshots.map(s => s.date)).size, data.snapshots.length);
  for (const row of data.snapshots) {
    assert.match(row.commit, /^[a-f0-9]{40}$/);
    assert.ok(total(row.loc) > 0);
  }
});

test('composition is one pie with exclusive shares summing to 100', () => {
  for (const row of composition(data.snapshots)) {
    assert.equal(row.spec.type, 'pie');
    assert.ok(Math.abs(row.spec.series[0].values.reduce((x, y) => x + y, 0) - 100) < 1e-8);
  }
});

test('client radar preserves actual LOC with stable axes and a common ceiling across months', () => {
  const variants = clientComposition(data.snapshots);
  const ceiling = variants[0].spec.radarMax;
  for (let i = 0; i < variants.length; i++) {
    const spec = variants[i].spec;
    assert.equal(spec.type, 'radar');
    assert.equal(spec.unit, 'LOC');
    assert.deepEqual(spec.labels, variants[0].spec.labels);
    assert.equal(spec.radarMax, ceiling);
    assert.equal(spec.series[0].values.reduce((a, b) => a + b, 0), total(data.snapshots[i].clientLoc));
    assert.ok(spec.series[0].values.every(v => v >= 0 && v <= ceiling));
  }
});

test('dependency focus includes incoming and outgoing neighbours and preserves every shown pair', () => {
  const focus = 'capsem-core';
  const view = dependencyView(latest, focus, 'all');
  const expected = new Set([focus, ...latest.edges.filter(e => e.from === focus || e.to === focus).flatMap(e => [e.from, e.to])]);
  assert.deepEqual(new Set(view.nodes.map(n => n.id)), expected);
  const pairs = new Set(latest.edges.filter(e => expected.has(e.from) && expected.has(e.to)).map(e => `${e.from}/${e.to}`));
  assert.deepEqual(new Set(view.links.map(e => `${e.from}/${e.to}`)), pairs);
});

test('kind filters cannot leak other kinds or invent crates', () => {
  for (const snapshot of data.snapshots) {
    const view = dependencyView(snapshot, '', 'runtime');
    assert.deepEqual(new Set(view.nodes.map(n => n.id)), new Set(snapshot.crates));
    for (const edge of view.links) {
      assert.equal(edge.label, 'runtime');
      assert.ok(snapshot.crates.includes(edge.from) && snapshot.crates.includes(edge.to));
    }
  }
});

test('every trend series has one value per labeled observation', () => {
  for (const chart of Object.values(statisticCharts(data))) {
    for (const series of chart.series) assert.equal(series.values.length, chart.labels.length, chart.id);
  }
});

test('release success rate includes failures/cancellations and leaves empty months missing', () => {
  const chart = statisticCharts({ ...data, releaseOutcomes: [
    { month: '2026-01', success: 1, failure: 1, cancelled: 2 },
    { month: '2026-02', success: 0, failure: 0, cancelled: 0 },
    { month: '2026-03', success: 3, failure: 0, cancelled: 0 },
  ] }).releases;
  assert.equal(chart.type, 'line');
  assert.equal(chart.unit, '%');
  assert.deepEqual(chart.series[0].values, [25, null, 100]);
});
