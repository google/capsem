import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { McpServerInfo, McpToolInfo } from '../types';

const mockServers: McpServerInfo[] = [
  {
    name: 'local',
    url: '',
    has_auth_credential: false,
    custom_header_count: 0,
    source: 'builtin',
    enabled: true,
    running: false,
    tool_count: 5,
    is_stdio: true,
  },
  {
    name: 'external',
    url: 'https://mcp.example.com',
    has_auth_credential: true,
    custom_header_count: 1,
    source: 'user',
    enabled: true,
    running: false,
    tool_count: 3,
    is_stdio: false,
  },
];

const mockTools: McpToolInfo[] = [
  { namespaced_name: 'local__http_get', original_name: 'http_get', description: 'HTTP GET', server_name: 'local', annotations: { title: null, read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true }, pin_hash: 'abc', pin_changed: false, permission_action: 'allow', permission_source: 'default' },
  { namespaced_name: 'external__search', original_name: 'search', description: 'Search', server_name: 'external', annotations: null, pin_hash: 'def', pin_changed: true, permission_action: 'ask', permission_source: 'profile_managed' },
];

vi.mock('../api', () => ({
  getMcpDefaultPermission: vi.fn(async () => ({ action: 'allow', source: 'default', rule_id: 'default.mcp' })),
  getMcpServers: vi.fn(async () => mockServers),
  getMcpTools: vi.fn(async (_profileId: string, serverId: string) =>
    mockTools.filter((tool) => tool.server_name === serverId)
  ),
  updateMcpDefaultPermission: vi.fn(async () => {}),
  updateMcpToolPermission: vi.fn(async () => {}),
  refreshMcpTools: vi.fn(async () => {}),
}));

describe('mcpStore', () => {
  let mcpStore: any;

  beforeEach(async () => {
    vi.resetModules();
    const mod = await import('../stores/mcp.svelte');
    mcpStore = mod.mcpStore;
  });

  it('loads servers and tools only', async () => {
    await mcpStore.load('co-work');

    expect(mcpStore.servers).toHaveLength(2);
    expect(mcpStore.servers[0].name).toBe('local');
    expect(mcpStore.servers[0].source).toBe('builtin');
    expect(mcpStore.profileId).toBe('co-work');

    expect(mcpStore.tools).toHaveLength(2);
    expect(mcpStore.defaultPermission.action).toBe('allow');
    expect(mcpStore.defaultPermission.rule_id).toBe('default.mcp');

    expect('policy' in mcpStore).toBe(false);

    expect(mcpStore.loading).toBe(false);

    expect(mcpStore.error).toBeNull();
  });

  it('computes derived state', async () => {
    await mcpStore.load('co-work');

    const grouped = mcpStore.toolsByServer;
    expect(grouped['local']).toHaveLength(1);

    expect(mcpStore.pinWarningCount).toBe(1);

    expect(mcpStore.totalTools).toBe(2);

    expect(mcpStore.runningCount).toBe(0);
  });

  it('does not expose retired policy or unsupported server mutation methods', () => {
    expect('setGlobalPolicy' in mcpStore).toBe(false);
    expect('toggleServer' in mcpStore).toBe(false);
    expect('addServer' in mcpStore).toBe(false);
    expect('removeServer' in mcpStore).toBe(false);
  });

  it('setDefaultPermission calls the profile-backed default rule API and reloads', async () => {
    await mcpStore.load('co-work');
    await mcpStore.setDefaultPermission('ask');
    const { updateMcpDefaultPermission } = await import('../api');
    expect(updateMcpDefaultPermission).toHaveBeenCalledWith('co-work', 'ask');
  });

  it('setToolPermission calls the profile-backed rule API and reloads', async () => {
    await mcpStore.load('co-work');
    await mcpStore.setToolPermission('local__http_get', 'ask');
    const { updateMcpToolPermission } = await import('../api');
    expect(updateMcpToolPermission).toHaveBeenCalledWith('co-work', 'local', 'http_get', 'ask');
  });

  it('refresh with server calls API', async () => {
    await mcpStore.load('co-work');
    await mcpStore.refresh('local');
    const { refreshMcpTools } = await import('../api');
    expect(refreshMcpTools).toHaveBeenCalledWith('co-work', 'local');
  });

  it('refresh without server refreshes each loaded server', async () => {
    await mcpStore.load('co-work');
    await mcpStore.refresh();
    const { refreshMcpTools } = await import('../api');
    expect(refreshMcpTools).toHaveBeenCalledWith('co-work', 'local');
    expect(refreshMcpTools).toHaveBeenCalledWith('co-work', 'external');
  });

  it('handles load error', async () => {
    const { getMcpServers } = await import('../api');
    (getMcpServers as any).mockRejectedValueOnce(new Error('boom'));
    await mcpStore.load('co-work');
    expect(mcpStore.error).toContain('boom');
  });

  it('requires an explicit profile before mutating MCP config', async () => {
    await expect(mcpStore.setToolPermission(mockTools[0], 'block')).rejects.toThrow('profile id');
    await expect(mcpStore.setDefaultPermission('block')).rejects.toThrow('profile id');
  });
});
