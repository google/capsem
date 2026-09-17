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
});
