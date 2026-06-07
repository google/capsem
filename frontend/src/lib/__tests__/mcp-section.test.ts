import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { SettingsResponse } from '../types/settings';

const apiMock = {
  getMcpServers: vi.fn(async () => []),
  getMcpTools: vi.fn(async () => []),
  getMcpPolicy: vi.fn(async () => ({
    global_policy: null,
    default_tool_permission: 'allow',
    blocked_servers: [],
    tool_permissions: {},
  })),
  setMcpServerEnabled: vi.fn(async () => {}),
  addMcpServer: vi.fn(async () => {}),
  removeMcpServer: vi.fn(async () => {}),
  setMcpGlobalPolicy: vi.fn(async () => {}),
  setMcpDefaultPermission: vi.fn(async () => {}),
  setMcpToolPermission: vi.fn(async () => {}),
  approveMcpTool: vi.fn(async () => {}),
  refreshMcpTools: vi.fn(async () => {}),
  reloadConfig: vi.fn(async () => ({ persisted: true, applied: true })),
};

vi.mock('../api', () => apiMock);

const { SettingsModel } = await import('../models/settings-model');
const { buildMockSettingsResponse } = await import('../mock-settings');
const { settingsStore } = await import('../stores/settings.svelte');
const { mcpStore } = await import('../stores/mcp.svelte');
const { default: McpSection } = await import('../components/settings/McpSection.svelte');

function responseWithLocalServer(enabled: boolean): SettingsResponse {
  const response = buildMockSettingsResponse();
  response.tree.push({
    kind: 'group',
    key: 'mcp',
    name: 'MCP Servers',
    description: 'Model Context Protocol servers available to AI agents',
    enabled_by: null,
    enabled: true,
    collapsed: false,
    children: [
      {
        kind: 'mcp_server',
        key: 'local',
        name: 'Local',
        description: 'Built-in local tools',
        transport: 'stdio',
        command: '/run/capsem-mcp-server',
        url: null,
        args: [],
        env: {},
        headers: {},
        builtin: true,
        enabled,
        source: 'default',
        corp_locked: false,
      },
    ],
  });
  return response;
}

describe('McpSection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    settingsStore.model = new SettingsModel(responseWithLocalServer(false));
    settingsStore.loading = false;
    settingsStore.error = null;
    mcpStore.servers = [];
    mcpStore.tools = [];
    mcpStore.policy = {
      global_policy: null,
      default_tool_permission: 'allow',
      blocked_servers: [],
      tool_permissions: {},
    };
  });

  it('keeps disabled local MCP visible and can re-enable it', async () => {
    render(McpSection);

    const toggle = screen.getByRole('switch', { name: /enable local/i });
    expect(toggle.getAttribute('aria-checked')).toBe('false');

    await fireEvent.click(toggle);

    await waitFor(() => {
      expect(apiMock.setMcpServerEnabled).toHaveBeenCalledWith('local', true);
    });
  });
});
