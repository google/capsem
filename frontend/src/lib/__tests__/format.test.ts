import { describe, it, expect } from 'vitest';
import {
  formatDuration,
  formatBytes,
  formatUptime,
  formatCost,
  formatTokens,
  truncate,
  fmtAge,
} from '../format';

describe('formatDuration', () => {
  it('formats sub-second as ms', () => {
    expect(formatDuration(0)).toBe('0ms');
    expect(formatDuration(500)).toBe('500ms');
    expect(formatDuration(999)).toBe('999ms');
  });

  it('formats >= 1s as seconds', () => {
    expect(formatDuration(1000)).toBe('1.0s');
    expect(formatDuration(1500)).toBe('1.5s');
    expect(formatDuration(60000)).toBe('60.0s');
  });
});

describe('formatBytes', () => {
  it('formats bytes', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(1023)).toBe('1023 B');
  });

  it('formats kilobytes', () => {
    expect(formatBytes(1024)).toBe('1.0 KB');
    expect(formatBytes(2048)).toBe('2.0 KB');
    expect(formatBytes(1536)).toBe('1.5 KB');
  });

  it('formats megabytes', () => {
    expect(formatBytes(1024 * 1024)).toBe('1.0 MB');
    expect(formatBytes(5 * 1024 * 1024)).toBe('5.0 MB');
  });
});

describe('formatUptime', () => {
  it('formats seconds', () => {
    expect(formatUptime(0)).toBe('0s');
    expect(formatUptime(30)).toBe('30s');
    expect(formatUptime(59)).toBe('59s');
  });

  it('formats minutes', () => {
    expect(formatUptime(60)).toBe('1m');
    expect(formatUptime(300)).toBe('5m');
    expect(formatUptime(3599)).toBe('59m');
  });

  it('formats hours', () => {
    expect(formatUptime(3600)).toBe('1h');
    expect(formatUptime(7200)).toBe('2h');
  });

  it('formats hours and minutes', () => {
    expect(formatUptime(5400)).toBe('1h 30m');
    expect(formatUptime(9000)).toBe('2h 30m');
  });
});

describe('formatCost', () => {
  it('formats zero', () => {
    expect(formatCost(0)).toBe('$0.00');
  });

  it('formats small amounts', () => {
    expect(formatCost(0.42)).toBe('$0.42');
    expect(formatCost(1.5)).toBe('$1.50');
  });

  it('formats larger amounts', () => {
    expect(formatCost(123.456)).toBe('$123.46');
  });
});

describe('formatTokens', () => {
  it('formats small numbers as-is', () => {
    expect(formatTokens(0)).toBe('0');
    expect(formatTokens(500)).toBe('500');
    expect(formatTokens(999)).toBe('999');
  });

  it('formats thousands as K', () => {
    expect(formatTokens(1000)).toBe('1.0K');
    expect(formatTokens(1200)).toBe('1.2K');
    expect(formatTokens(50000)).toBe('50.0K');
  });

  it('formats millions as M', () => {
    expect(formatTokens(1_000_000)).toBe('1.0M');
    expect(formatTokens(1_500_000)).toBe('1.5M');
  });
});

describe('truncate', () => {
  it('returns short strings unchanged', () => {
    expect(truncate('abc', 10)).toBe('abc');
    expect(truncate('', 5)).toBe('');
  });

  it('truncates long strings with ellipsis', () => {
    expect(truncate('hello world', 5)).toBe('hello...');
    expect(truncate('abcdef', 3)).toBe('abc...');
  });

  it('handles exact boundary', () => {
    expect(truncate('abc', 3)).toBe('abc');
  });
});

describe('fmtAge', () => {
  it('returns empty for empty input', () => {
    expect(fmtAge('')).toBe('');
  });

  it('returns "just now" for recent timestamps', () => {
    const now = new Date().toISOString();
    expect(fmtAge(now)).toBe('just now');
  });

  it('returns minutes for recent past', () => {
    const fiveMinAgo = new Date(Date.now() - 5 * 60_000).toISOString();
    expect(fmtAge(fiveMinAgo)).toBe('5m ago');
  });

  it('returns hours for older timestamps', () => {
    const twoHoursAgo = new Date(Date.now() - 2 * 3600_000).toISOString();
    expect(fmtAge(twoHoursAgo)).toBe('2h ago');
  });
});
