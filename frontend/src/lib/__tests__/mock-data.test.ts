import { describe, it, expect } from 'vitest';
import {
  mockVMs, getVM,
  mockModelStats, mockToolCalls, mockNetworkEvents, mockFileEvents,
  mockLogEntries,
} from '../mock.ts';
import type { MockLogEntry } from '../mock.ts';

// ---------------------------------------------------------------------------
// VM data
// ---------------------------------------------------------------------------

describe('mockVMs', () => {
  it('has 5 VMs', () => {
    expect(mockVMs).toHaveLength(5);
  });

  it('each VM has required fields', () => {
    for (const vm of mockVMs) {
      expect(vm.id).toBeTruthy();
      expect(vm.name).toBeTruthy();
      expect(['running', 'stopped', 'booting', 'error']).toContain(vm.status);
      expect(vm.ram).toBeGreaterThan(0);
      expect(vm.cpus).toBeGreaterThan(0);
    }
  });

  it('getVM returns matching VM', () => {
    expect(getVM('vm-1')?.name).toBe('dev-sandbox');
    expect(getVM('nonexistent')).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// Stats data
// ---------------------------------------------------------------------------

describe('mockModelStats', () => {
  it('has entries', () => {
    expect(mockModelStats.length).toBeGreaterThan(0);
  });

  it('each entry has valid token counts', () => {
    for (const m of mockModelStats) {
      expect(m.provider).toBeTruthy();
      expect(m.model).toBeTruthy();
      expect(m.inputTokens).toBeGreaterThanOrEqual(0);
      expect(m.outputTokens).toBeGreaterThanOrEqual(0);
      expect(m.cacheTokens).toBeGreaterThanOrEqual(0);
      expect(m.estimatedCostUsd).toBeGreaterThanOrEqual(0);
      expect(m.callCount).toBeGreaterThan(0);
    }
  });
});

describe('mockToolCalls', () => {
  it('has entries', () => {
    expect(mockToolCalls.length).toBeGreaterThan(0);
  });

  it('each entry has valid fields', () => {
    for (const tc of mockToolCalls) {
      expect(tc.id).toBeTruthy();
      expect(tc.tool).toBeTruthy();
      expect(tc.server).toBeTruthy();
      expect(tc.durationMs).toBeGreaterThanOrEqual(0);
      expect(tc.timestamp).toBeTruthy();
    }
  });

  it('has unique IDs', () => {
    const ids = mockToolCalls.map(tc => tc.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});

describe('mockNetworkEvents', () => {
  it('has both allowed and denied events', () => {
    expect(mockNetworkEvents.some(e => e.decision === 'allowed')).toBe(true);
    expect(mockNetworkEvents.some(e => e.decision === 'denied')).toBe(true);
  });

  it('denied events have status 0', () => {
    for (const e of mockNetworkEvents.filter(e => e.decision === 'denied')) {
      expect(e.status).toBe(0);
      expect(e.durationMs).toBe(0);
    }
  });

  it('allowed events have positive status', () => {
    for (const e of mockNetworkEvents.filter(e => e.decision === 'allowed')) {
      expect(e.status).toBeGreaterThan(0);
    }
  });
});

describe('mockFileEvents', () => {
  it('has all operation types', () => {
    const ops = new Set(mockFileEvents.map(e => e.operation));
    expect(ops.has('created')).toBe(true);
    expect(ops.has('modified')).toBe(true);
    expect(ops.has('deleted')).toBe(true);
  });

  it('deleted events have null size', () => {
    for (const e of mockFileEvents.filter(e => e.operation === 'deleted')) {
      expect(e.sizeBytes).toBeNull();
    }
  });
});

// ---------------------------------------------------------------------------
// Log data
// ---------------------------------------------------------------------------

describe('mockLogEntries', () => {
  it('has entries', () => {
    expect(mockLogEntries.length).toBeGreaterThan(0);
  });

  it('has all log levels', () => {
    const levels = new Set(mockLogEntries.map(e => e.level));
    expect(levels.has('info')).toBe(true);
    expect(levels.has('warn')).toBe(true);
    expect(levels.has('error')).toBe(true);
  });

  it('each entry has valid fields', () => {
    for (const e of mockLogEntries) {
      expect(e.id).toBeTruthy();
      expect(e.timestamp).toBeTruthy();
      expect(['info', 'warn', 'error']).toContain(e.level);
      expect(e.source).toBeTruthy();
      expect(e.message).toBeTruthy();
    }
  });

  it('has unique IDs', () => {
    const ids = mockLogEntries.map(e => e.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('timestamps are in chronological order', () => {
    for (let i = 1; i < mockLogEntries.length; i++) {
      expect(new Date(mockLogEntries[i].timestamp).getTime())
        .toBeGreaterThanOrEqual(new Date(mockLogEntries[i - 1].timestamp).getTime());
    }
  });
});

// ---------------------------------------------------------------------------
// Log filtering (pure logic extracted for testing)
// ---------------------------------------------------------------------------

describe('log filtering', () => {
  function filterLogs(
    entries: MockLogEntry[],
    level: 'all' | 'info' | 'warn' | 'error',
    source: string,
    search: string,
  ): MockLogEntry[] {
    let result = entries;
    if (level !== 'all') result = result.filter(e => e.level === level);
    if (source !== 'all') result = result.filter(e => e.source === source);
    if (search.trim()) {
      const q = search.trim().toLowerCase();
      result = result.filter(e => e.message.toLowerCase().includes(q) || e.source.toLowerCase().includes(q));
    }
    return result;
  }

  it('returns all entries with no filters', () => {
    expect(filterLogs(mockLogEntries, 'all', 'all', '')).toHaveLength(mockLogEntries.length);
  });

  it('filters by level', () => {
    const warns = filterLogs(mockLogEntries, 'warn', 'all', '');
    expect(warns.length).toBeGreaterThan(0);
    expect(warns.every(e => e.level === 'warn')).toBe(true);
  });

  it('filters by source', () => {
    const source = 'vm::boot';
    const result = filterLogs(mockLogEntries, 'all', source, '');
    expect(result.length).toBeGreaterThan(0);
    expect(result.every(e => e.source === source)).toBe(true);
  });

  it('filters by search text (message)', () => {
    const result = filterLogs(mockLogEntries, 'all', 'all', 'kernel');
    expect(result.length).toBeGreaterThan(0);
    expect(result.every(e => e.message.toLowerCase().includes('kernel'))).toBe(true);
  });

  it('filters by search text (source)', () => {
    const result = filterLogs(mockLogEntries, 'all', 'all', 'mitm');
    expect(result.length).toBeGreaterThan(0);
    expect(result.every(e => e.source.toLowerCase().includes('mitm') || e.message.toLowerCase().includes('mitm'))).toBe(true);
  });

  it('combines level + search', () => {
    const result = filterLogs(mockLogEntries, 'warn', 'all', 'denied');
    expect(result.length).toBeGreaterThan(0);
    expect(result.every(e => e.level === 'warn')).toBe(true);
    expect(result.every(e => e.message.toLowerCase().includes('denied'))).toBe(true);
  });

  it('returns empty for non-matching search', () => {
    const result = filterLogs(mockLogEntries, 'all', 'all', 'zzz_no_match_zzz');
    expect(result).toHaveLength(0);
  });
});

