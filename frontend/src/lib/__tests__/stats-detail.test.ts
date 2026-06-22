import { describe, expect, it } from 'vitest';
import {
  compactJsonForDisplay,
  detailPayloadSections,
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
