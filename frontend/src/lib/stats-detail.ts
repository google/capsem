export type DetailPayloadSection = {
  key: string;
  label: string;
  value: unknown;
  lang: string;
};

const DETAIL_PAYLOAD_KEYS = new Set([
  'request_headers',
  'response_headers',
  'request_body',
  'response_body',
  'context_json',
]);

const DETAIL_STRUCTURED_KEYS = new Set([
  'rule_json',
  'event_json',
]);

const DETAIL_BODY_METADATA_KEYS = new Set([
  'request_body_content_type',
  'request_body_original_bytes',
  'request_body_stored_bytes',
  'request_body_truncated',
  'request_body_hash',
  'response_body_content_type',
  'response_body_original_bytes',
  'response_body_stored_bytes',
  'response_body_truncated',
  'response_body_hash',
]);

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
  return Object.entries(obj).filter(([key, value]) => (
    isPresent(value)
    && !DETAIL_PAYLOAD_KEYS.has(key)
    && !DETAIL_STRUCTURED_KEYS.has(key)
    && !DETAIL_BODY_METADATA_KEYS.has(key)
    && !DETAIL_HIDDEN_KEYS.has(key)
  ));
}

export function detailPayloadSections(obj: Record<string, unknown>): DetailPayloadSection[] {
  return Object.entries(obj)
    .filter(([key, value]) => DETAIL_PAYLOAD_KEYS.has(key) && isPresent(value))
    .map(([key, value]) => ({
      key,
      label: labelForDetailKey(key),
      value,
      lang: detailPayloadLang(key, value),
    }));
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
  if (typeof value === 'object') return JSON.stringify(value);
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
