import { formatBytes } from './format';

export type DetailPayloadSection = {
  key: string;
  label: string;
  value: unknown;
  lang: string;
  /** Whether there are bytes to render, as opposed to provenance alone. */
  hasContent: boolean;
};

// The sections the detail pane renders as payloads. `*_body` values are
// fetched from GET /vms/{id}/bodies/{event_id} when an event is expanded --
// the list views carry only the index metadata beside them.
//
// `payload_body` is the security-rule matched event. It was metadata-only
// while there was no route to read the bytes with; it is a body like any
// other now, and renders through the same section as the rest.
const DETAIL_PAYLOAD_KEYS = new Set([
  'request_headers',
  'response_headers',
  'request_body',
  'response_body',
  'payload_body',
  'context_json',
]);

const DETAIL_STRUCTURED_KEYS = new Set([
  'rule_json',
]);

export const BODY_DIRECTIONS = ['request', 'response', 'payload'] as const;

// Everything the body index and the body route say *about* a body, as opposed
// to the body. These render as the small grid above each payload section and
// are kept out of the generic field list, where they would bury the event's
// own columns under fifteen rows of provenance.
const DETAIL_BODY_METADATA_SUFFIXES = [
  'content_type',
  'original_bytes',
  'stored_bytes',
  'truncated',
  'hash',
  // From the route rather than the index: how it was encoded for transport,
  // how much of it this response carried, and whether it had to cut it.
  'encoding',
  'shown_bytes',
  'truncated_for_transport',
];

const DETAIL_BODY_METADATA_KEYS = new Set(
  BODY_DIRECTIONS.flatMap(direction =>
    DETAIL_BODY_METADATA_SUFFIXES.map(suffix => `${direction}_body_${suffix}`),
  ),
);

const DETAIL_HIDDEN_KEYS = new Set([
  'substitution_ref',
  'credential_ref',
]);

export function isPresent(value: unknown): boolean {
  if (value == null) return false;
  if (typeof value === 'string') return value.trim().length > 0;
  if (Array.isArray(value)) return value.length > 0;
  if (typeof value === 'object') return Object.keys(value).length > 0;
  return true;
}

export function labelForDetailKey(key: string): string {
  return key
    .split('_')
    .map(part => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ');
}

export function visibleDetailEntries(obj: Record<string, unknown>): [string, unknown][] {
  return Object.entries(obj)
    .filter(([key]) => (
      !DETAIL_PAYLOAD_KEYS.has(key)
      && !DETAIL_STRUCTURED_KEYS.has(key)
      && !DETAIL_BODY_METADATA_KEYS.has(key)
      && !DETAIL_HIDDEN_KEYS.has(key)
    ))
    .map(([key, value]) => [key, stripEmptyDetailValues(value)] as [string, unknown])
    .filter(([, value]) => isPresent(value));
}

const BODY_SECTION_KEYS = BODY_DIRECTIONS.map(direction => `${direction}_body`);

// The sections the detail pane renders, content first and then the bodies the
// index names but whose bytes are not here.
//
// That second pass is not a nicety. A section used to exist only when its
// content did, and every metadata key is filtered out of the generic field
// grid, so a body whose fetch failed -- or one the upstream sent empty --
// dropped its content type, its sizes, its truncation flag and its hash out of
// the pane entirely. The pane said nothing rather than "there was a body here
// and these are its dimensions", which is the difference between an event with
// no body and an event whose body could not be read.
//
// The hash is the marker, as it is for the metadata rows: an index row exists
// for this direction or it does not.
export function detailPayloadSections(obj: Record<string, unknown>): DetailPayloadSection[] {
  const sections: DetailPayloadSection[] = Object.entries(obj)
    .filter(([key, value]) => DETAIL_PAYLOAD_KEYS.has(key) && isPresent(value))
    .map(([key, value]) => ({
      key,
      label: labelForDetailKey(key),
      value,
      lang: detailPayloadLang(key, value),
      hasContent: true,
    }));

  for (const key of BODY_SECTION_KEYS) {
    if (sections.some(section => section.key === key)) continue;
    if (!isPresent(obj[`${key}_hash`])) continue;
    sections.push({
      key,
      label: labelForDetailKey(key),
      value: null,
      lang: 'text',
      hasContent: false,
    });
  }
  return sections;
}

export function detailPayloadLang(key: string, value: unknown): string {
  if (key.endsWith('_headers')) return 'http';
  if (key === 'context_json') return 'json';
  const content = normalizePayloadContent(typeof value === 'string' ? value : JSON.stringify(value));
  const trimmed = content.trim();
  if (trimmed.startsWith('{') || trimmed.startsWith('[')) {
    try {
      JSON.parse(trimmed);
      return 'json';
    } catch {
      return 'text';
    }
  }
  return 'text';
}

export function formatDetailValue(value: unknown): string {
  if (value == null) return 'NULL';
  if (typeof value === 'object') return JSON.stringify(stripEmptyDetailValues(value));
  return String(value);
}

function parseMaybeJson(value: unknown): unknown {
  if (typeof value !== 'string') return value;
  const normalized = normalizePayloadContent(value);
  const trimmed = normalized.trim();
  if (!trimmed) return value;
  if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) return normalized;
  try {
    return JSON.parse(trimmed);
  } catch {
    return normalized;
  }
}

function stripEmptyDetailValues(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value
      .map(item => stripEmptyDetailValues(item))
      .filter(isPresent);
  }
  if (value && typeof value === 'object') {
    const compact: Record<string, unknown> = {};
    for (const [key, child] of Object.entries(value)) {
      const stripped = stripEmptyDetailValues(child);
      if (isPresent(stripped)) compact[key] = stripped;
    }
    return compact;
  }
  return value;
}

export function compactJsonForDisplay(value: unknown): unknown {
  return stripEmptyDetailValues(parseMaybeJson(value));
}

export function normalizePayloadContent(content: string): string {
  const trimmed = content.trim();
  if (!trimmed) return content;
  if (
    (trimmed.startsWith('{') || trimmed.startsWith('['))
    && (trimmed.includes('\\"') || trimmed.includes('\\n') || trimmed.includes('\\t'))
  ) {
    const unescaped = trimmed
      .replace(/\\n/g, '\n')
      .replace(/\\r/g, '\r')
      .replace(/\\t/g, '\t')
      .replace(/\\"/g, '"');
    try {
      JSON.parse(unescaped);
      return unescaped;
    } catch {
      return content;
    }
  }
  return content;
}

// The metadata beside one body: what it is, how big it was, how much of it the
// archive kept, how much of that this page is showing, and the hash a reader
// checks it against.
//
// Every row drops out when its field is absent, including Truncated -- which
// would otherwise read "no" whether the body was whole or there was no body
// row at all, and render a section holding a lone "TRUNCATED no". The hash is
// the marker that an index row exists; no rows means no metadata, and the
// caller should render no section.
//
// Truncated and Showing are two different statements and stay two rows.
// Truncated is the capture: the upstream sent more than Capsem kept, and the
// rest is gone. Showing is this response: the route sent a prefix and the rest
// is one larger request away. Merging them would tell a reviewer evidence was
// lost when it is sitting in the archive.
export function payloadSectionMeta(
  section: { key: string },
  obj: Record<string, unknown>,
): { label: string; value: string }[] {
  const prefix = section.key;
  const hash = metaText(obj[`${prefix}_hash`]);
  return [
    { label: 'Content Type', value: metaText(obj[`${prefix}_content_type`]) },
    { label: 'Original', value: metaBytes(obj[`${prefix}_original_bytes`]) },
    { label: 'Stored', value: metaBytes(obj[`${prefix}_stored_bytes`]) },
    { label: 'Truncated', value: hash ? (metaNumber(obj[`${prefix}_truncated`]) === 1 ? 'yes' : 'no') : '' },
    { label: 'Showing', value: transportNote(prefix, obj) },
    { label: 'Encoding', value: metaText(obj[`${prefix}_encoding`]) },
    { label: 'Hash', value: hash },
  ].filter(row => row.value.length > 0);
}

// "first 1 MB of 3 MB", and nothing at all when the whole body came back --
// a row saying the response was complete is a row on every body forever.
export function transportNote(prefix: string, obj: Record<string, unknown>): string {
  if (obj[`${prefix}_truncated_for_transport`] !== true) return '';
  const shown = metaBytes(obj[`${prefix}_shown_bytes`]);
  const stored = metaBytes(obj[`${prefix}_stored_bytes`]);
  if (!shown) return '';
  return stored ? `first ${shown} of ${stored}` : `first ${shown}`;
}

function metaText(value: unknown): string {
  return value == null ? '' : String(value);
}

function metaNumber(value: unknown): number {
  const n = Number(value ?? 0);
  return Number.isFinite(n) ? n : 0;
}

function metaBytes(value: unknown): string {
  if (!isPresent(value)) return '';
  return formatBytes(metaNumber(value));
}
