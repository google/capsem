// Test-facing settings fixture. The settings tree itself is generated from the
// backend contract.

import {
  MOCK_MCP_SERVERS,
  MOCK_MCP_TOOLS,
  buildMockTree,
  mockSettings,
  recomputeEnabled,
} from './mock-settings.generated';
import type { SettingsResponse } from './types/settings';

export {
  MOCK_MCP_SERVERS,
  MOCK_MCP_TOOLS,
  buildMockTree,
  mockSettings,
  recomputeEnabled,
};

export function buildMockSettingsResponse(): SettingsResponse {
  return {
    tree: buildMockTree(),
    issues: [],
  };
}
