import { it, expect } from 'vitest';
import type { TerminalState } from '@capsem/sdk';
import { SessionProjection } from '../terminal/session-projection';
import { MAX_STREAM_FRAME_BYTES } from '../terminal/stream-protocol';

const state = (revision: number, cols = 80, rows = 24, reset = false): TerminalState => ({
  revision, cols, rows, reset, title: '', icon_name: '', bell_count: 0, events: [],
  cursor: { row: 0, col: 0, hidden: false },
  modes: { alternate_screen: false, application_cursor: false, application_keypad: false, bracketed_paste: false, mouse_protocol: 'None', mouse_encoding: 'Default' },
});

it('applies session dimensions only after the preceding write has completed', async () => {
  const actions: string[] = [];
  let finish = () => {};
  const projection = new SessionProjection({
    reset: () => actions.push('reset'),
    resize: (cols, rows) => actions.push(`${cols}x${rows}`),
    write: (bytes, done) => { actions.push(new TextDecoder().decode(bytes)); finish = done; },
  }, () => actions.push('resync'));
  projection.state(state(0, 80, 24, true));
  projection.output(new TextEncoder().encode('old-grid'));
  projection.state(state(1, 101, 33, true));
  expect(actions).toEqual(['reset', '80x24', 'old-grid']);
  finish();
  await Promise.resolve();
  expect(actions).toEqual(['reset', '80x24', 'old-grid', 'reset', '101x33']);
});

it('resyncs instead of rendering out-of-order state or dropping display bytes', () => {
  let resyncs = 0;
  const projection = new SessionProjection({ reset: () => {}, resize: () => {}, write: () => {} }, () => resyncs++);
  projection.state(state(5, 80, 24, true));
  projection.state(state(4));
  expect(resyncs).toBe(1);
  projection.reconnect();
  projection.state(state(0, 80, 24, true));
  projection.output(new Uint8Array(1)); // deliberately held renderer
  projection.output(new Uint8Array(MAX_STREAM_FRAME_BYTES * 32));
  expect(resyncs).toBe(1);
  projection.output(new Uint8Array(1));
  expect(resyncs).toBe(2);
});
