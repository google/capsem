// Test/mock settings wrapper.
// The setting defaults and tree come from mock-settings.generated.ts, which is
// derived from builder settings fixtures. Keep hand-authored data here limited
// to frontend-only fixtures outside the generated tree.

import {
  buildMockTree as buildGeneratedMockTree,
  recomputeEnabled,
} from './mock-settings.generated';
import type {
  ConfigIssue,
  PolicyConfig,
  SecurityPreset,
  SettingsNode,
  SettingsResponse,
} from './types/settings';

export {
  MOCK_MCP_POLICY,
  MOCK_MCP_SERVERS,
  MOCK_MCP_TOOLS,
  mockSettings,
  recomputeEnabled,
} from './mock-settings.generated';

export const MOCK_PRESETS: SecurityPreset[] = [
  {
    id: 'medium',
    name: 'Medium',
    description: 'Allow read-only web, all search engines, MCP tools without confirmation.',
    settings: {
      'security.web.allow_read': true,
      'security.web.allow_write': false,
      'security.services.search.google.allow': true,
      'security.services.search.bing.allow': true,
      'security.services.search.duckduckgo.allow': true,
    },
    mcp: { default_tool_permission: 'allow' },
  },
  {
    id: 'high',
    name: 'High',
    description: 'Block all web access, selective search only, stricter MCP policies.',
    settings: {
      'security.web.allow_read': false,
      'security.web.allow_write': false,
      'security.services.search.google.allow': true,
      'security.services.search.bing.allow': false,
      'security.services.search.duckduckgo.allow': false,
    },
    mcp: { default_tool_permission: 'warn' },
  },
];

export const MOCK_POLICY: PolicyConfig = {
  mcp: {
    ask_prod_issue: {
      on: 'mcp.request',
      if: 'method == "tools/call" && arguments.issue == "prod"',
      decision: 'ask',
      priority: 20,
      reason: 'Require approval before production issue tools run',
    },
  },
  http: {
    block_openai_github: {
      on: 'http.request',
      if: 'request.host == "github.com" && request.path.matches("^/openai(/|$)")',
      decision: 'block',
      priority: 10,
      reason: 'Block OpenAI organization GitHub paths',
    },
  },
  dns: {},
  model: {},
  hook: {},
};

const MOCK_ISSUES: ConfigIssue[] = [
  {
    id: 'ai.anthropic.api_key',
    severity: 'warning',
    message: 'No Anthropic API key configured. Claude Code will not be able to authenticate.',
    docs_url: 'https://console.anthropic.com/settings/keys',
  },
  {
    id: 'ai.google.api_key',
    severity: 'warning',
    message: 'No Google AI API key configured. Gemini CLI will not be able to authenticate.',
    docs_url: 'https://aistudio.google.com/apikey',
  },
  {
    id: 'ai.openai.api_key',
    severity: 'warning',
    message: 'No OpenAI API key configured. Codex CLI will not be able to authenticate.',
    docs_url: 'https://platform.openai.com/api-keys',
  },
];

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

export function buildMockTree(): SettingsNode[] {
  recomputeEnabled();
  return buildGeneratedMockTree();
}

export function buildMockSettingsResponse(): SettingsResponse {
  return {
    tree: buildMockTree(),
    issues: clone(MOCK_ISSUES),
    presets: clone(MOCK_PRESETS),
    policy: clone(MOCK_POLICY),
  };
}
