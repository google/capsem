import { describe, expect, it } from 'vitest';
import type { EventBody } from '../api';
import {
  bodyContent,
  createDetailLoader,
  safeEventId,
  shownBytes,
  withFetchedBodies,
  withIndexMetadata,
  type DetailRow,
  type DetailSelection,
} from '../event-bodies';

function body(overrides: Partial<EventBody> = {}): EventBody {
  return {
    event_id: '0123456789ab',
    source_table: 'net_events',
    direction: 'response',
    content_type: 'application/json',
    original_bytes: 16,
    stored_bytes: 16,
    truncated: false,
    truncated_for_transport: false,
    body_hash: 'blake3:abc',
    encoding: 'utf8',
    content: '{"answer":"yes"}',
    ...overrides,
  };
}

/** A pane that records what it was told, in order. */
function recordingView() {
  const shown: DetailSelection[] = [];
  const errors: (string | null)[] = [];
  return {
    shown,
    errors,
    get selection() {
      return shown.at(-1) ?? null;
    },
    get error() {
      return errors.at(-1) ?? null;
    },
    view: {
      show: (selection: DetailSelection) => { shown.push(selection); },
      setError: (message: string | null) => { errors.push(message); },
    },
  };
}

/** A fetch whose resolution the test decides when to trigger. */
function deferred<T>() {
  let settle!: (value: T) => void;
  let fail!: (reason: unknown) => void;
  const promise = new Promise<T>((resolve, reject) => {
    settle = resolve;
    fail = reject;
  });
  return { promise, settle, fail };
}

const NO_INDEX_ROWS = { indexRowsFor: () => [] as DetailRow[] };

describe('event id validation', () => {
  it('accepts only twelve lowercase hex characters', () => {
    expect(safeEventId('0123456789ab')).toBe('0123456789ab');
    for (const rejected of ['0123456789a', '0123456789abc', '0123456789AB', '../', '', null, undefined, 42]) {
      expect(safeEventId(rejected)).toBeNull();
    }
  });
});

describe('detail selection sequencing', () => {
  it('drops a fetch that lands after the user has selected another event', async () => {
    const pane = recordingView();
    const first = deferred<EventBody[]>();
    const second = deferred<EventBody[]>();
    const pending = [first, second];
    const showDetail = createDetailLoader(pane.view, {
      ...NO_INDEX_ROWS,
      fetchBodies: () => pending.shift()!.promise,
    });

    const a = showDetail('http', { event_id: 'aaaaaaaaaaaa' });
    const b = showDetail('http', { event_id: 'bbbbbbbbbbbb' });

    second.settle([body({ event_id: 'bbbbbbbbbbbb', content: 'second' })]);
    await b;
    first.settle([body({ event_id: 'aaaaaaaaaaaa', content: 'first' })]);
    await a;

    expect(pane.selection?.data.event_id).toBe('bbbbbbbbbbbb');
    expect(pane.selection?.data.response_body).toBe('second');
  });

  it('drops a fetch that lands after a selection with no event id', async () => {
    // The regression. The token used to be taken after the `!eventId` return,
    // so this second selection never superseded the first, and the first
    // fetch painted its bodies over a row that has no bodies at all.
    const pane = recordingView();
    const first = deferred<EventBody[]>();
    const showDetail = createDetailLoader(pane.view, {
      ...NO_INDEX_ROWS,
      fetchBodies: () => first.promise,
    });

    const a = showDetail('http', { event_id: 'aaaaaaaaaaaa' });
    await showDetail('dns', { qname: 'example.test' });

    first.settle([body({ event_id: 'aaaaaaaaaaaa', content: 'first' })]);
    await a;

    expect(pane.selection?.type).toBe('dns');
    expect(pane.selection?.data.qname).toBe('example.test');
    expect(pane.selection?.data.response_body).toBeUndefined();
  });

  it('does not let a failed fetch complain about an event the user has left', async () => {
    const pane = recordingView();
    const first = deferred<EventBody[]>();
    const showDetail = createDetailLoader(pane.view, {
      ...NO_INDEX_ROWS,
      fetchBodies: () => first.promise,
    });

    const a = showDetail('http', { event_id: 'aaaaaaaaaaaa' });
    await showDetail('dns', { qname: 'example.test' });

    first.fail(new Error('gateway went away'));
    await a;

    expect(pane.error).toBeNull();
  });

  it('withdraws a previous event\'s error banner, including for a row with no event id', async () => {
    // The second regression: the early return skipped the reset, so a failed
    // event's banner sat over the next, unrelated selection.
    const pane = recordingView();
    const showDetail = createDetailLoader(pane.view, {
      ...NO_INDEX_ROWS,
      fetchBodies: () => Promise.reject(new Error('gateway went away')),
    });

    await showDetail('http', { event_id: 'aaaaaaaaaaaa' });
    expect(pane.error).toBe('gateway went away');

    await showDetail('dns', { qname: 'example.test' });
    expect(pane.error).toBeNull();

    await showDetail('http', { event_id: 'aaaaaaaaaaaa' });
    expect(pane.error).toBe('gateway went away');
  });

  it('shows the row immediately and the bodies when they arrive', async () => {
    const pane = recordingView();
    const showDetail = createDetailLoader(pane.view, {
      indexRowsFor: () => [
        { direction: 'response', content_type: 'application/json', original_bytes: 16, stored_bytes: 16, truncated: 0, body_hash: 'blake3:abc' },
      ],
      fetchBodies: async () => [body()],
    });

    await showDetail('http', { event_id: '0123456789ab', status_code: 200 });

    // Row, then row plus index metadata, then row plus bytes: the pane is
    // never blank while the fetch is out.
    expect(pane.shown).toHaveLength(3);
    expect(pane.shown[0].data.response_body).toBeUndefined();
    expect(pane.shown[1].data.response_body_hash).toBe('blake3:abc');
    expect(pane.shown[2].data.response_body).toBe('{"answer":"yes"}');
  });
});

describe('body content', () => {
  it('keeps the index metadata when the fetch fails, so the pane can still describe the body', async () => {
    const pane = recordingView();
    const showDetail = createDetailLoader(pane.view, {
      indexRowsFor: () => [
        { direction: 'payload', content_type: 'application/json', original_bytes: 2400, stored_bytes: 2400, truncated: 0, body_hash: 'blake3:abc' },
      ],
      fetchBodies: () => Promise.reject(new Error('gateway went away')),
    });

    await showDetail('security', { event_id: '0123456789ab' });

    expect(pane.selection?.data.payload_body_hash).toBe('blake3:abc');
    expect(pane.selection?.data.payload_body_original_bytes).toBe(2400);
    expect(pane.selection?.data.payload_body).toBeUndefined();
  });

  it('reports what the route sent apart from what the capture kept', () => {
    const row = withFetchedBodies({}, [
      body({ truncated: false, truncated_for_transport: true, stored_bytes: 3145728, content: 'abcd' }),
    ]);
    expect(row.response_body_truncated).toBe(0);
    expect(row.response_body_truncated_for_transport).toBe(true);
    expect(row.response_body_shown_bytes).toBe(4);
    expect(row.response_body_stored_bytes).toBe(3145728);
  });

  it('measures shown bytes as decoded bytes, not as characters', () => {
    // Four three-byte characters, cut to two by the route.
    expect(shownBytes(body({ content: '水水' }))).toBe(6);
    // base64 of four bytes is eight characters with two of padding.
    expect(shownBytes(body({ encoding: 'base64', content: '//4AAQ==' }))).toBe(4);
  });

  it('does not render non-text bodies as text', () => {
    const binary = body({ encoding: 'base64', content: '//4AAQ==' });
    expect(bodyContent(binary)).toBe('[binary body, 4 bytes, not text]');
    expect(bodyContent(body())).toBe('{"answer":"yes"}');
  });

  it('ignores directions the detail pane has no section for', () => {
    const row = withFetchedBodies({}, [body({ direction: 'stdout', content: 'hello' })]);
    expect(row.stdout_body).toBeUndefined();
    expect(withIndexMetadata({}, [{ direction: 'stdout', body_hash: 'blake3:abc' }])).toEqual({});
  });
});
