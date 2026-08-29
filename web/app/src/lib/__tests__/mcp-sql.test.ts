import { describe, expect, it } from 'vitest';
import {
  TRACES_SQL,
  TOOL_CALL_LEDGER_WHERE,
  TOOL_COUNT_SQL,
  TOOLS_OVER_TIME_SQL,
  TOOLS_STATS_SQL,
  TOOLS_TOP_SERVERS_SQL,
  TOOLS_TOP_TOOLS_SQL,
  TOOLS_UNIFIED_MINIMAL_SQL,
  TOOLS_UNIFIED_SEARCH_SQL,
  TOOLS_UNIFIED_SQL,
} from '../sql';

describe('MCP stats SQL', () => {
  it('uses the canonical tool_calls ledger predicate for headline and tool-list queries', () => {
    const queries = [
      TOOL_COUNT_SQL,
      TOOLS_STATS_SQL,
      TOOLS_TOP_TOOLS_SQL,
      TOOLS_TOP_SERVERS_SQL,
      TOOLS_OVER_TIME_SQL,
      TOOLS_UNIFIED_SQL,
      TOOLS_UNIFIED_MINIMAL_SQL,
      TOOLS_UNIFIED_SEARCH_SQL,
    ];

    for (const query of queries) {
      expect(query).toContain(TOOL_CALL_LEDGER_WHERE.trim());
    }
    expect(TOOLS_UNIFIED_SQL).toContain('FROM tool_calls');
    expect(TOOLS_UNIFIED_MINIMAL_SQL).toContain('FROM tool_calls');
    expect(TOOLS_UNIFIED_SQL).not.toContain('FROM mcp_calls');
    expect(TOOLS_UNIFIED_MINIMAL_SQL).not.toContain('FROM mcp_calls');
  });
});

describe('Model trace SQL', () => {
  it('does not hide model traces that have no parsed token usage yet', () => {
    expect(TRACES_SQL).toContain('COUNT(mc.id) as call_count');
    expect(TRACES_SQL).toContain('total_tool_calls');
    expect(TRACES_SQL).not.toMatch(/HAVING\s+total_input_tokens\s*\+\s*total_output_tokens\s*>\s*0/i);
  });
});
