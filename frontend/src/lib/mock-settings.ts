// Test-facing settings fixture. The settings tree itself is generated from the
// backend contract; only runtime provider status is hand-authored here.

import {
  MOCK_MCP_SERVERS,
  MOCK_MCP_TOOLS,
  buildMockTree,
  mockSettings,
  recomputeEnabled,
} from './mock-settings.generated';
import type { ProviderStatus, SettingsResponse } from './types/settings';

export {
  MOCK_MCP_SERVERS,
  MOCK_MCP_TOOLS,
  buildMockTree,
  mockSettings,
  recomputeEnabled,
};

const MOCK_CREDENTIAL_REF = `credential:blake3:${'0'.repeat(64)}`;

export const MOCK_PROVIDER_STATUS: ProviderStatus[] = [
  {
    id: 'openai',
    name: 'OpenAI',
    protocol: 'openai',
    url: 'https://api.openai.com/v1',
    aliases: ['api.openai.com'],
    listen_ports: [443],
    allowed_remote_targets: ['api.openai.com:443'],
    discovery: {
      observed_at: '2026-06-06T12:00:00Z',
      source: 'credential_broker',
      event_type: 'file.event',
      confidence: 0.96,
      credential_ref: MOCK_CREDENTIAL_REF,
      trace_id: 'abc123def456',
    },
    corp_blocked: false,
  },
  {
    id: 'anthropic',
    name: 'Anthropic',
    protocol: 'anthropic',
    url: 'https://api.anthropic.com',
    aliases: ['api.anthropic.com'],
    listen_ports: [443],
    allowed_remote_targets: ['api.anthropic.com:443'],
    discovery: null,
    corp_blocked: false,
  },
  {
    id: 'ollama',
    name: 'Ollama',
    protocol: 'ollama',
    url: 'http://127.0.0.1:11434',
    aliases: ['localhost', '127.0.0.1', 'host.docker.internal', 'local.ollama'],
    listen_ports: [11434],
    allowed_remote_targets: ['127.0.0.1:11434', 'local.ollama:11434'],
    discovery: null,
    corp_blocked: false,
  },
];

export function buildMockSettingsResponse(): SettingsResponse {
  return {
    tree: buildMockTree(),
    issues: [],
    providers: MOCK_PROVIDER_STATUS,
  };
}
