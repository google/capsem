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
  const shown: (DetailSelection | null)[] = [];
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
      show: (selection: DetailSelection | null) => { shown.push(selection); },
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
    const loader = createDetailLoader(pane.view, {
      ...NO_INDEX_ROWS,
      fetchBodies: () => pending.shift()!.promise,
    });

    const a = loader.show('http', { event_id: 'aaaaaaaaaaaa' });
    const b = loader.show('http', { event_id: 'bbbbbbbbbbbb' });

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
    const loader = createDetailLoader(pane.view, {
      ...NO_INDEX_ROWS,
      fetchBodies: () => first.promise,
    });

    const a = loader.show('http', { event_id: 'aaaaaaaaaaaa' });
    await loader.show('dns', { qname: 'example.test' });

    first.settle([body({ event_id: 'aaaaaaaaaaaa', content: 'first' })]);
    await a;

    expect(pane.selection?.type).toBe('dns');
    expect(pane.selection?.data.qname).toBe('example.test');
    expect(pane.selection?.data.response_body).toBeUndefined();
  });

  it('does not let a failed fetch complain about an event the user has left', async () => {
    const pane = recordingView();
    const first = deferred<EventBody[]>();
    const loader = createDetailLoader(pane.view, {
      ...NO_INDEX_ROWS,
      fetchBodies: () => first.promise,
    });

    const a = loader.show('http', { event_id: 'aaaaaaaaaaaa' });
    await loader.show('dns', { qname: 'example.test' });

    first.fail(new Error('gateway went away'));
    await a;

    expect(pane.error).toBeNull();
  });

  it('withdraws a previous event\'s error banner, including for a row with no event id', async () => {
    // The second regression: the early return skipped the reset, so a failed
    // event's banner sat over the next, unrelated selection.
    const pane = recordingView();
    const loader = createDetailLoader(pane.view, {
      ...NO_INDEX_ROWS,
      fetchBodies: () => Promise.reject(new Error('gateway went away')),
    });

    await loader.show('http', { event_id: 'aaaaaaaaaaaa' });
    expect(pane.error).toBe('gateway went away');

    await loader.show('dns', { qname: 'example.test' });
    expect(pane.error).toBeNull();

    await loader.show('http', { event_id: 'aaaaaaaaaaaa' });
    expect(pane.error).toBe('gateway went away');
  });

  it('does not let a dismissed pane be reopened by its own fetch', async () => {
    // The fourth face of the same bug, and the one the contract test could not
    // see: closing the pane writes `null`, not an object, so `dismiss` taking
    // no token meant an in-flight fetch landed, found itself current, and
    // reopened the pane on the event the user had just closed.
    const pane = recordingView();
    const first = deferred<EventBody[]>();
    const loader = createDetailLoader(pane.view, {
      ...NO_INDEX_ROWS,
      fetchBodies: () => first.promise,
    });

    const a = loader.show('http', { event_id: 'aaaaaaaaaaaa' });
    loader.dismiss();

    first.settle([body({ event_id: 'aaaaaaaaaaaa', content: 'first' })]);
    await a;

    expect(pane.selection).toBeNull();
  });

  it('does not let a dismissed pane be given an error by its own fetch', async () => {
    const pane = recordingView();
    const first = deferred<EventBody[]>();
    const loader = createDetailLoader(pane.view, {
      ...NO_INDEX_ROWS,
      fetchBodies: () => first.promise,
    });

    const a = loader.show('http', { event_id: 'aaaaaaaaaaaa' });
    loader.dismiss();

    first.fail(new Error('gateway went away'));
    await a;

    expect(pane.error).toBeNull();
  });

  it('withdraws a standing error banner when the pane is dismissed', async () => {
    const pane = recordingView();
    const loader = createDetailLoader(pane.view, {
      ...NO_INDEX_ROWS,
      fetchBodies: () => Promise.reject(new Error('gateway went away')),
    });

    await loader.show('http', { event_id: 'aaaaaaaaaaaa' });
    expect(pane.error).toBe('gateway went away');

    loader.dismiss();
    expect(pane.error).toBeNull();
    expect(pane.selection).toBeNull();
  });

  it('shows the row immediately and the bodies when they arrive', async () => {
    const pane = recordingView();
    const loader = createDetailLoader(pane.view, {
      indexRowsFor: () => [
        { direction: 'response', content_type: 'application/json', original_bytes: 16, stored_bytes: 16, truncated: 0, body_hash: 'blake3:abc' },
      ],
      fetchBodies: async () => [body()],
    });

    await loader.show('http', { event_id: '0123456789ab', status_code: 200 });

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
    const loader = createDetailLoader(pane.view, {
      indexRowsFor: () => [
        // Shaped as the stats-detail route sends it, which always names the table.
        { source_table: 'security_rule_events', direction: 'payload', content_type: 'application/json', original_bytes: 2400, stored_bytes: 2400, truncated: 0, body_hash: 'blake3:abc' },
      ],
      fetchBodies: () => Promise.reject(new Error('gateway went away')),
    });

    await loader.show('security', { event_id: '0123456789ab' });

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

  it('renders a guest command\'s output, which the archive has held all along', () => {
    // `update_exec_event` stages "stdout" and "stderr" as string literals, so
    // a grep for the read-side `BodyDirection::Stdout` finds nothing and the
    // bodies were there regardless. The pane had no section, so an exec event
    // showed its exit code and nothing it printed.
    const row = withFetchedBodies({}, [
      body({
        direction: 'stdout',
        source_table: 'exec_events',
        content_type: 'text/plain',
        content: 'Compiling capsem-core\n',
        // capsem-process truncates guest output to 1 KiB before the writer
        // sees it, and `original_bytes` is what the command actually produced.
        stored_bytes: 1024,
        original_bytes: 40960,
        truncated: true,
      }),
      body({ direction: 'stderr', source_table: 'exec_events', content: 'warning: unused\n' }),
    ]);

    expect(row.stdout_body).toBe('Compiling capsem-core\n');
    expect(row.stderr_body).toBe('warning: unused\n');
    // The excerpt must not read as the whole output.
    expect(row.stdout_body_truncated).toBe(1);
    expect(row.stdout_body_original_bytes).toBe(40960);
    expect(row.stdout_body_stored_bytes).toBe(1024);
  });

  it('ignores directions the detail pane has no section for', () => {
    // Still pinned, against a direction that genuinely has none -- the point
    // is that an unknown direction is dropped rather than rendered under a
    // key nothing displays, not that `stdout` in particular is unknown.
    const row = withFetchedBodies({}, [body({ direction: 'trailer', content: 'hello' })]);
    expect(row.trailer_body).toBeUndefined();
    expect(withIndexMetadata({}, [{ direction: 'trailer', body_hash: 'blake3:abc' }])).toEqual({});
  });

  it('shows the rule match\'s payload, not a decision\'s or an ask\'s for the same event', () => {
    // All three archive a `payload` for one event id and the route returns
    // every body an event has. The pane that renders a payload is the rule
    // match's; the others must not overwrite it, whatever order they arrive in.
    const payload = (source_table: string, body_hash: string) =>
      body({ source_table, direction: 'payload', body_hash, content: `{"from":"${source_table}"}` });
    for (const order of [
      ['security_rule_events', 'security_decision_events', 'security_ask_events'],
      ['security_decision_events', 'security_ask_events', 'security_rule_events'],
    ]) {
      const bodies = order.map(table => payload(table, `blake3:${table}`));
      const row = withFetchedBodies({}, bodies);
      expect(row.payload_body).toBe('{"from":"security_rule_events"}');
      expect(row.payload_body_hash).toBe('blake3:security_rule_events');

      const indexed = withIndexMetadata(
        {},
        order.map(table => ({ source_table: table, direction: 'payload', body_hash: `blake3:${table}` })),
      );
      expect(indexed.payload_body_hash).toBe('blake3:security_rule_events');
    }
  });

  it('still shows request and response bodies from whichever table holds them', () => {
    // Only the payload is shared across tables; the other directions stay as
    // they were, so a model call's request is not filtered away.
    const row = withFetchedBodies({}, [
      body({ source_table: 'model_calls', direction: 'request', content: '{"model":"m"}' }),
    ]);
    expect(row.request_body).toBe('{"model":"m"}');
  });
});
