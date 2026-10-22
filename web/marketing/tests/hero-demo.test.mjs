import assert from 'node:assert/strict';
import test from 'node:test';
import { HERO_DEMO, HERO_TERMINAL_SCRIPT as steps } from '../src/lib/data.ts';
import { dockTarget, spring } from '../src/lib/hero-dock.ts';

test('Scale creates each VM before its prompt and stays in the GUI', () => {
  const scale = steps.slice(steps.findIndex(step => step.chapter === 3));
  assert.ok(!scale.some(step => step.surface === 'tui' || step.surface === 'mcp'));
  assert.ok(!JSON.stringify(scale).includes('capsem_list'));
  assert.deepEqual(scale.flatMap(step => step.scale === undefined ? [] : [step.scale]), [2, 3, 4, 5, 6]);
  assert.deepEqual(scale.flatMap(step => step.launch === undefined ? [] : [step.launch]), [0, 1, 2, 3, 4, 5]);
  let session = 0, count = 2, view = 'sessions', name = '';
  for (const step of scale) {
    if (step.guiSession !== undefined) session = step.guiSession;
    if (step.guiView !== undefined) view = step.guiView;
    if (view === 'create' && step.type) name = step.type;
    if (view === 'boot' && step.scale !== undefined) {
      assert.equal(name, HERO_DEMO.scale.sessions[session].name);
      assert.equal(step.scale, count + 1);
    }
    if (step.scale !== undefined) count = step.scale;
    if (step.launch !== undefined) {
      assert.ok(step.launch < count);
      assert.equal(view, 'terminal');
    }
  }
  assert.equal(scale.at(-1).guiView, 'sessions');
});

test('TUI creation holds on confirmation and the new running session', () => {
  const start = steps.findIndex(step => step.overlay === 'create');
  const flow = steps.slice(start, steps.findIndex(step => step.chapter === 3));
  assert.equal(flow[0].type, 'Email productivity');
  assert.ok(flow.some(step => step.key === 'Enter' && step.wait >= 500));
  assert.ok(flow.some(step => step.overlay === 'sessions' && step.wait >= 1000));
});

test('dock magnification and nudge are continuous across the icon center', () => {
  assert.equal(dockTarget(0, 1).scale, 1.7);
  assert.equal(dockTarget(111, 1).scale, 1);
  assert.equal(dockTarget(0, 0).scale, 1);
  // A sign-only nudge jumped ~40px across this tiny pointer movement.
  assert.ok(Math.abs(dockTarget(-.001, 1).x - dockTarget(.001, 1).x) < .01);
  assert.equal(dockTarget(-40, 1).scale, dockTarget(40, 1).scale);
  assert.equal(dockTarget(-40, 1).x, -dockTarget(40, 1).x);
});

test('dock springs settle without overshoot at different frame rates', () => {
  for (const dt of [8, 16, 33, 50]) {
    let value = 1, velocity = 0;
    for (let time = 0; time < 2000; time += dt) {
      ({ value, velocity } = spring(value, velocity, 1.7, dt));
      assert.ok(Number.isFinite(value) && value >= 1 && value < 1.701);
    }
    assert.ok(Math.abs(value - 1.7) < .001);
    for (let time = 0; time < 2000; time += dt) ({ value, velocity } = spring(value, velocity, 1, dt));
    assert.ok(Math.abs(value - 1) < .001);
  }
});
