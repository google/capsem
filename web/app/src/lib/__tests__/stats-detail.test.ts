import { describe, expect, it } from 'vitest';
import {
  compactJsonForDisplay,
  detailPayloadSections,
  formatDetailValue,
  payloadSectionMeta,
  visibleDetailEntries,
} from '../stats-detail';

describe('stats detail helpers', () => {
  it('removes null-heavy security event projections instead of preserving empty shells', () => {
    const compact = compactJsonForDisplay({
      event_id: 'abc123def456',
      http: null,
      mcp: null,
      model: null,
      file: {
        read_path: null,
        write_path: '.gemini/antigravity-cli/conversations',
        write_name: 'conversations',
        write_content: null,
      },
      detections: [],
      decision: {
        effective: 'allow',
        reason: '',
      },
      plugin_executions: [],
    });

    expect(compact).toEqual({
      event_id: 'abc123def456',
      file: {
        write_path: '.gemini/antigravity-cli/conversations',
        write_name: 'conversations',
      },
      decision: {
        effective: 'allow',
      },
    });
  });

  it('keeps generic event fields focused on present ledger values only', () => {
    const fields = visibleDetailEntries({
      event_id: '87868a03279a',
      credential_ref: 'credential:blake3:not-for-display',
      response_body: '{"ok":true}',
      response_body_hash: 'blake3:abc',
      request_body_original_bytes: 42,
      empty: '',
      absent: null,
      status_code: 200,
    });

    expect(fields).toEqual([
      ['event_id', '87868a03279a'],
      ['status_code', 200],
    ]);
  });

  it('renders nested object fields without null-only branches', () => {
    const fields = visibleDetailEntries({
      event_id: 'file123456789',
      file: {
        read_path: null,
        read_name: null,
        write_path: '.gemini/antigravity-cli/conversations',
        write_name: 'conversations',
        write_content: null,
      },
      http: null,
      detections: [],
    });

    expect(fields).toEqual([
      ['event_id', 'file123456789'],
      [
        'file',
        {
          write_path: '.gemini/antigravity-cli/conversations',
          write_name: 'conversations',
        },
      ],
    ]);
    expect(formatDetailValue(fields[1][1])).toBe(
      '{"write_path":".gemini/antigravity-cli/conversations","write_name":"conversations"}',
    );
  });

  it('classifies payload sections without duplicating them into the field grid', () => {
    const sections = detailPayloadSections({
      event_id: '87868a03279a',
      request_headers: 'host: example.test',
      response_body: '{\\n  \\"ok\\": true\\n}',
      context_json: '{"source":"credential_broker"}',
      response_body_hash: 'blake3:def',
    });

    expect(sections.map(section => [section.key, section.lang])).toEqual([
      ['request_headers', 'http'],
      ['response_body', 'json'],
      ['context_json', 'json'],
    ]);
  });
});

describe('archived body metadata', () => {
  it('reports what the archive holds for a body the index names', () => {
    const rows = payloadSectionMeta({ key: 'payload_body' }, {
      payload_body_content_type: 'application/json',
      payload_body_original_bytes: 2400,
      payload_body_stored_bytes: 2400,
      payload_body_truncated: 0,
      payload_body_hash: `blake3:${'a'.repeat(64)}`,
    });
    expect(rows.map(row => row.label)).toEqual([
      'Content Type',
      'Original',
      'Stored',
      'Truncated',
      'Hash',
    ]);
    expect(rows.find(row => row.label === 'Truncated')?.value).toBe('no');
  });

  it('says a body was truncated only when there is a body to say it about', () => {
    // A rule match whose payload the archive never stored has no index row and
    // so no metadata. Truncated used to answer "no" for it anyway, which is
    // how the Matched Event section rendered a lone "TRUNCATED no".
    expect(payloadSectionMeta({ key: 'payload_body' }, { rule_id: 'profiles.rules.x' })).toEqual([]);
    const truncated = payloadSectionMeta({ key: 'payload_body' }, {
      payload_body_truncated: 1,
      payload_body_hash: `blake3:${'b'.repeat(64)}`,
    });
    expect(truncated.find(row => row.label === 'Truncated')?.value).toBe('yes');
  });

  it('reports what the route sent separately from what the capture kept', () => {
    const rows = payloadSectionMeta({ key: 'response_body' }, {
      response_body_stored_bytes: 3 * 1024 * 1024,
      response_body_original_bytes: 3 * 1024 * 1024,
      response_body_truncated: 0,
      response_body_truncated_for_transport: true,
      response_body_shown_bytes: 1024 * 1024,
      response_body_encoding: 'utf8',
      response_body_hash: `blake3:${'c'.repeat(64)}`,
    });
    // The whole body is in the archive; this page is showing a prefix of it.
    // Reading "Truncated no" beside "Showing first 1.0 MB of 3.0 MB" is the point:
    // a reviewer must be able to tell a lost body from a paged one.
    expect(rows.find(row => row.label === 'Truncated')?.value).toBe('no');
    expect(rows.find(row => row.label === 'Showing')?.value).toBe('first 1.0 MB of 3.0 MB');
    expect(rows.find(row => row.label === 'Encoding')?.value).toBe('utf8');
  });

  it('says nothing about transport when the whole body came back', () => {
    const rows = payloadSectionMeta({ key: 'response_body' }, {
      response_body_stored_bytes: 11,
      response_body_shown_bytes: 11,
      response_body_truncated_for_transport: false,
      response_body_hash: `blake3:${'d'.repeat(64)}`,
    });
    expect(rows.map(row => row.label)).not.toContain('Showing');
  });

  it('renders a security payload as a body section, not as metadata alone', () => {
    const sections = detailPayloadSections({
      rule_id: 'profiles.rules.x',
      payload_body: '{"model":{"provider":"ollama"}}',
    });
    expect(sections.map(section => [section.key, section.lang, section.hasContent])).toEqual([
      ['payload_body', 'json', true],
    ]);
  });

  it('keeps the section when the bytes could not be fetched', () => {
    // A section used to exist only when its content did, and every metadata
    // key is filtered out of the generic field grid -- so a failed fetch took
    // the content type, both sizes, the truncation flag and the hash out of
    // the pane with it, and the pane said nothing at all about a body it knew
    // was there. The index metadata alone is still worth rendering.
    const sections = detailPayloadSections({
      rule_id: 'profiles.rules.x',
      payload_body_content_type: 'application/json',
      payload_body_original_bytes: 2400,
      payload_body_stored_bytes: 2400,
      payload_body_truncated: 0,
      payload_body_hash: `blake3:${'e'.repeat(64)}`,
    });
    expect(sections.map(section => [section.key, section.hasContent])).toEqual([
      ['payload_body', false],
    ]);
    expect(payloadSectionMeta(sections[0], {
      payload_body_content_type: 'application/json',
      payload_body_original_bytes: 2400,
      payload_body_stored_bytes: 2400,
      payload_body_truncated: 0,
      payload_body_hash: `blake3:${'e'.repeat(64)}`,
    }).map(row => row.label)).toEqual(['Content Type', 'Original', 'Stored', 'Truncated', 'Hash']);
  });

  it('keeps the section when the upstream sent a zero-length body', () => {
    // `isPresent('')` is false, so an empty body is indistinguishable from a
    // missing one by content alone. The index row says one was stored.
    const sections = detailPayloadSections({
      response_body: '',
      response_body_content_type: 'text/plain',
      response_body_original_bytes: 0,
      response_body_stored_bytes: 0,
      response_body_truncated: 0,
      response_body_hash: `blake3:${'f'.repeat(64)}`,
    });
    expect(sections.map(section => [section.key, section.hasContent])).toEqual([
      ['response_body', false],
    ]);
  });

  it('says nothing about a direction the index never named', () => {
    // No hash means no index row means no body: an empty section here would
    // assert a body that never existed.
    expect(detailPayloadSections({ rule_id: 'profiles.rules.x' })).toEqual([]);
  });

  it('does not duplicate a body that has both content and metadata', () => {
    const sections = detailPayloadSections({
      response_body: '{"ok":true}',
      response_body_hash: `blake3:${'a'.repeat(64)}`,
    });
    expect(sections).toHaveLength(1);
    expect(sections[0].hasContent).toBe(true);
  });
});
