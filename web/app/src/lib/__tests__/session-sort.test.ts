import { describe, expect, it } from 'vitest';
import { compareSessions, sessionTokens, type SessionSortKey, type SortableSession } from '../models/session-sort';

const sessions = [
  { id: 'a', name: 'Ollama', status: 'Stopped', uptime_secs: 10, total_input_tokens: 2, total_output_tokens: 8, total_estimated_cost: .2 },
  { id: 'b', name: 'Antigravity', status: 'Running', uptime_secs: 30, total_input_tokens: 7, total_output_tokens: 1, total_estimated_cost: .5 },
  { id: 'c', name: null, status: 'Suspended', uptime_secs: null, total_input_tokens: null, total_output_tokens: 9, total_estimated_cost: null },
] satisfies readonly SortableSession[];

function order(key: SessionSortKey, direction: 'asc' | 'desc' = 'asc') {
  return [...sessions].sort((a, b) => compareSessions(a, b, key, direction)).map(session => session.id);
}

describe('session table sorting', () => {
  it('shows output-only usage and distinguishes unknown usage from zero', () => {
    expect(sessionTokens(sessions[2])).toBe(9);
    expect(sessionTokens({total_input_tokens:null,total_output_tokens:null})).toBeNull();
    expect(sessionTokens({total_input_tokens:0,total_output_tokens:null})).toBe(0);
  });
  it('sorts the displayed total input plus output tokens in both directions', () => {
    expect(order('tokens')).toEqual(['b', 'c', 'a']);
    expect(order('tokens', 'desc')).toEqual(['a', 'c', 'b']);
  });

  it('sorts estimated cost numerically and treats absent values as zero', () => {
    expect(order('cost')).toEqual(['c', 'a', 'b']);
    expect(order('cost', 'desc')).toEqual(['b', 'a', 'c']);
  });

  it('preserves name fallback, status and uptime sorting', () => {
    expect(order('name')).toEqual(['b', 'c', 'a']);
    expect(order('status')).toEqual(['b', 'a', 'c']);
    expect(order('uptime')).toEqual(['c', 'a', 'b']);
    expect(order('uptime', 'desc')).toEqual(['b', 'a', 'c']);
  });

  it('leaves equal values in their original order', () => {
    expect(compareSessions(sessions[0], { ...sessions[0], id: 'd' }, 'tokens', 'asc')).toBe(0);
    expect(sessions.map(session => session.id)).toEqual(['a', 'b', 'c']);
  });
});
