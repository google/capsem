/**
 * Loading one event's archived bodies into the detail pane.
 *
 * This lived inside `StatsView.svelte` and had two ordering bugs in it that no
 * test could have caught, because the only tests a component has here read its
 * source as text. The rules below are logic, not markup -- which selection is
 * the open one, what a stale response is allowed to do, what an error banner
 * outlives -- so they live where they can be exercised.
 *
 * The rule the bugs broke: **a selection supersedes everything before it, from
 * the moment it is made.** Not from the moment it turns out to have an event
 * id, which is what "bump the token after the early return" meant in practice:
 * pick an event, then pick a row that has no event id, and the first fetch
 * still believed it was current and painted its bodies over the second row
 * when it landed. The stale error banner outlived its event the same way.
 *
 * Closing the pane is a selection too -- of nothing. It was the fourth face of
 * the same bug: a tab switch or the close button set the selection to `null`
 * without taking a token, so a fetch already out landed, found itself current,
 * and reopened the pane on the event the user had just dismissed.
 */

import type { EventBody } from './api';
import { BODY_DIRECTIONS } from './stats-detail';

export type DetailRow = Record<string, any>;
export type DetailSelection = { type: string; data: Record<string, unknown> };

/** What the loader does to the pane. The component supplies the writes. */
export type DetailView = {
  /** Show this selection, or close the pane when it is `null`. */
  show(selection: DetailSelection | null): void;
  /** Say why the bodies are missing, or withdraw a previous complaint. */
  setError(message: string | null): void;
};

/** The pane's two verbs. Nothing else may write the selection. */
export type DetailLoader = {
  show(type: string, row: DetailRow): Promise<void>;
  /** Close the pane, and with it whatever it had in flight. */
  dismiss(): void;
};

export type DetailSources = {
  /** The body index rows the list response already carried for this event. */
  indexRowsFor(eventId: string): DetailRow[];
  /** The bytes, from `GET /vms/{id}/bodies/{event_id}`. */
  fetchBodies(eventId: string): Promise<EventBody[]>;
};

/** An event id as the ledger writes one, or nothing. */
export function safeEventId(value: unknown): string | null {
  const id = value == null ? '' : String(value);
  return /^[0-9a-f]{12}$/.test(id) ? id : null;
}

function isBodyDirection(value: string): value is (typeof BODY_DIRECTIONS)[number] {
  return (BODY_DIRECTIONS as readonly string[]).includes(value);
}

/**
 * How many bytes of the archived body this response actually carried.
 *
 * The route says that it cut one, not where. For text that is the encoded
 * length of what came back; for base64 it is what those characters decode to.
 */
export function shownBytes(body: EventBody): number {
  if (body.encoding === 'base64') {
    const padding = (body.content.match(/=+$/)?.[0].length) ?? 0;
    return Math.max(0, Math.floor((body.content.length * 3) / 4) - padding);
  }
  return new TextEncoder().encode(body.content).length;
}

/**
 * Bytes that are not text are not rendered as text. The metadata beside this
 * says what they are; base64 through a syntax highlighter is noise dressed as
 * evidence.
 */
export function bodyContent(body: EventBody): string {
  if (body.encoding !== 'base64') return body.content;
  return `[binary body, ${shownBytes(body)} bytes, not text]`;
}

/** The row plus what the body index says was captured for it. */
export function withIndexMetadata(row: DetailRow, indexRows: DetailRow[]): DetailRow {
  const enriched: DetailRow = { ...row };
  for (const indexRow of indexRows) {
    const direction = indexRow.direction == null ? '' : String(indexRow.direction);
    if (!isBodyDirection(direction)) continue;
    enriched[`${direction}_body_content_type`] = indexRow.content_type;
    enriched[`${direction}_body_original_bytes`] = indexRow.original_bytes;
    enriched[`${direction}_body_stored_bytes`] = indexRow.stored_bytes;
    enriched[`${direction}_body_truncated`] = indexRow.truncated;
    enriched[`${direction}_body_hash`] = indexRow.body_hash;
  }
  return enriched;
}

/** The row plus the bytes the route sent and what it did to them. */
export function withFetchedBodies(row: DetailRow, bodies: EventBody[]): DetailRow {
  const withBodies: DetailRow = { ...row };
  for (const body of bodies) {
    if (!isBodyDirection(body.direction)) continue;
    const key = `${body.direction}_body`;
    withBodies[key] = bodyContent(body);
    withBodies[`${key}_content_type`] = body.content_type;
    withBodies[`${key}_original_bytes`] = body.original_bytes;
    withBodies[`${key}_stored_bytes`] = body.stored_bytes;
    withBodies[`${key}_truncated`] = body.truncated ? 1 : 0;
    withBodies[`${key}_truncated_for_transport`] = body.truncated_for_transport;
    withBodies[`${key}_shown_bytes`] = shownBytes(body);
    withBodies[`${key}_encoding`] = body.encoding;
    withBodies[`${key}_hash`] = body.body_hash;
  }
  return withBodies;
}

/**
 * The pane's only way to open or close a row.
 *
 * Every row in every tab, every tab switch and the close button go through
 * these two. A path that wrote the selection directly would be a selection no
 * fetch in flight knew about, and the stale response would win -- which is one
 * bug with four faces. Three of them were writing an object; the fourth was
 * writing `null`, and closing the pane while a fetch was out let that fetch
 * reopen it on the event the user had just dismissed.
 *
 * `dismiss` therefore takes a token like any other selection. Closing the pane
 * *is* a selection: the selection of nothing.
 */
export function createDetailLoader(view: DetailView, sources: DetailSources): DetailLoader {
  let current = 0;

  /** Supersede everything in flight and withdraw any standing complaint. */
  function begin(): number {
    const token = ++current;
    view.setError(null);
    return token;
  }

  return {
    async show(type: string, row: DetailRow): Promise<void> {
      // First, and before anything can return. See the module comment.
      const token = begin();
      view.show({ type, data: row });

      const eventId = safeEventId(row.event_id);
      if (!eventId) return;

      const enriched = withIndexMetadata(row, sources.indexRowsFor(eventId));
      view.show({ type, data: enriched });

      let fetched: EventBody[];
      try {
        fetched = await sources.fetchBodies(eventId);
      } catch (e) {
        // Only the open selection may complain. A failure for an event the
        // user has already left is not this event's failure.
        if (token === current) view.setError(e instanceof Error ? e.message : 'Failed to load event bodies');
        return;
      }
      if (token !== current) return;

      view.show({ type, data: withFetchedBodies(enriched, fetched) });
    },

    dismiss(): void {
      begin();
      view.show(null);
    },
  };
}
