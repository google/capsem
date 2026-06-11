import { describe, expect, it } from 'vitest';
import {
  MCP_USER_TOOL_CALL_WHERE,
  TRACES_SQL,
  TOOL_COUNT_SQL,
  TOOLS_OVER_TIME_SQL,
  TOOLS_STATS_SQL,
  TOOLS_TOP_SERVERS_SQL,
  TOOLS_TOP_TOOLS_SQL,
  TOOLS_UNIFIED_SEARCH_SQL,
  TOOLS_UNIFIED_SQL,
} from '../sql';

describe('MCP stats SQL', () => {
  it('uses the user MCP call predicate for headline and tool-list queries', () => {
    const queries = [
      TOOL_COUNT_SQL,
      TOOLS_STATS_SQL,
      TOOLS_TOP_TOOLS_SQL,
      TOOLS_TOP_SERVERS_SQL,
      TOOLS_OVER_TIME_SQL,
      TOOLS_UNIFIED_SQL,
      TOOLS_UNIFIED_SEARCH_SQL,
    ];

    for (const query of queries) {
      expect(query).toContain(MCP_USER_TOOL_CALL_WHERE.trim());
    }
  });
});

describe('Model trace SQL', () => {
  it('does not hide model traces that have no parsed token usage yet', () => {
    expect(TRACES_SQL).toContain('COUNT(mc.id) as call_count');
    expect(TRACES_SQL).toContain('total_tool_calls');
    expect(TRACES_SQL).not.toMatch(/HAVING\s+total_input_tokens\s*\+\s*total_output_tokens\s*>\s*0/i);
  });
});
