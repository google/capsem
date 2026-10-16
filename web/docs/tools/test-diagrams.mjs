import assert from 'node:assert/strict';
import { test } from 'node:test';
import { introduction, systemOverview, securityEngine, investigation } from '../src/lib/overview-diagrams.ts';
import { securityProcesses, securityNetwork } from '../src/lib/diagrams.ts';
import { diagramLegend, nodeMeaning, semantics } from '../../diagrams/semantics.ts';
import { SECURITY_DIAGRAM } from '../../marketing/src/lib/data.ts';

const diagrams = [introduction, systemOverview, securityEngine, investigation, securityProcesses, securityNetwork, SECURITY_DIAGRAM];

test('distinct diagram meanings use distinct Tailwind color families', () => {
  for (const diagram of diagrams) {
    const legend = diagramLegend(diagram);
    const families = legend.map(meaning => semantics[meaning].family);
    assert.equal(new Set(families).size, legend.length, `${diagram.title}: meanings share a color family`);
    for (const node of diagram.nodes) {
      assert.ok(legend.includes(nodeMeaning(node)), `${diagram.title}: ${node.label} has no legend entry`);
    }
  }
  assert.notEqual(semantics.engine.family, semantics.limited.family);
  assert.notEqual(semantics.vm.family, semantics.processing.family);
});

test('diagram connections resolve to unique nodes', () => {
  for (const diagram of diagrams) {
    const ids = new Set(diagram.nodes.map(node => node.id));
    assert.equal(ids.size, diagram.nodes.length, diagram.title);
    for (const link of diagram.links) {
      assert.ok(ids.has(link.from) && ids.has(link.to), `${diagram.title}: ${link.from} → ${link.to}`);
    }
  }
});

test('external services stay outside per-VM and processing boundaries', () => {
  for (const diagram of diagrams) {
    for (const node of diagram.nodes.filter(node => ['upstream', 'tools'].includes(node.id))) {
      for (const boundary of diagram.nodes.filter(node => node.boundary)) {
        const overlaps = node.x < boundary.x + boundary.width && node.x + (node.width ?? 220) > boundary.x
          && node.y < boundary.y + boundary.height && node.y + (node.height ?? 72) > boundary.y;
        assert.ok(!overlaps, `${diagram.title}: ${node.label} is inside ${boundary.label}`);
      }
    }
  }
});
