import { describe, expect, it } from 'vitest';
import {
  NET_EVENTS_ALL_SQL,
  NET_EVENTS_SEARCH_SQL,
  TOOLS_UNIFIED_SEARCH_SQL,
  TOOLS_UNIFIED_SQL,
  TRACE_TOOL_CALLS_SQL,
} from '../sql';

describe('session SQL policy fields', () => {
  it('projects MCP policy metadata for tool views', () => {
    for (const sql of [
      TRACE_TOOL_CALLS_SQL,
      TOOLS_UNIFIED_SQL,
      TOOLS_UNIFIED_SEARCH_SQL,
    ]) {
      expect(sql).toContain('policy_mode');
      expect(sql).toContain('policy_action');
      expect(sql).toContain('policy_rule');
      expect(sql).toContain('policy_reason');
      expect(sql).toContain('trace_id');
    }
  });

  it('projects network policy metadata for event views', () => {
    for (const sql of [NET_EVENTS_ALL_SQL, NET_EVENTS_SEARCH_SQL]) {
      expect(sql).toContain('policy_mode');
      expect(sql).toContain('policy_action');
      expect(sql).toContain('policy_rule');
      expect(sql).toContain('policy_reason');
      expect(sql).toContain('trace_id');
    }
  });
});
