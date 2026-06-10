import { describe, expect, it } from 'vitest';
import {
  MCP_USER_TOOL_CALL_WHERE,
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
