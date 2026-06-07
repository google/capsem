import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { McpServerInfo, McpToolInfo, McpPolicyInfo } from '../types';

const mockServers: McpServerInfo[] = [
  {
    name: 'builtin',
    url: '',
    has_bearer_token: false,
    custom_header_count: 0,
    source: 'default',
    enabled: true,
    running: true,
    tool_count: 5,
    is_stdio: false,
  },
  {
    name: 'external',
    url: 'https://mcp.example.com',
    has_bearer_token: true,
    custom_header_count: 1,
    source: 'user',
    enabled: true,
    running: false,
    tool_count: 3,
    is_stdio: false,
  },
];

const mockTools: McpToolInfo[] = [
  { namespaced_name: 'builtin__http_get', original_name: 'http_get', description: 'HTTP GET', server_name: 'builtin', annotations: { title: null, read_only_hint: true, destructive_hint: false, idempotent_hint: true, open_world_hint: true }, pin_hash: 'abc', approved: true, pin_changed: false },
  { namespaced_name: 'external__search', original_name: 'search', description: 'Search', server_name: 'external', annotations: null, pin_hash: 'def', approved: false, pin_changed: true },
];

const mockPolicy: McpPolicyInfo = {
  global_policy: 'allow',
  default_tool_permission: 'allow',
  blocked_servers: [],
  tool_permissions: {},
};

vi.mock('../api', () => ({
  getMcpServers: vi.fn(async () => mockServers),
  getMcpTools: vi.fn(async () => mockTools),
  getMcpPolicy: vi.fn(async () => mockPolicy),
  setMcpServerEnabled: vi.fn(async () => {}),
  addMcpServer: vi.fn(async () => {}),
  removeMcpServer: vi.fn(async () => {}),
  setMcpGlobalPolicy: vi.fn(async () => {}),
  setMcpDefaultPermission: vi.fn(async () => {}),
  setMcpToolPermission: vi.fn(async () => {}),
  approveMcpTool: vi.fn(async () => {}),
  refreshMcpTools: vi.fn(async () => {}),
}));

describe('mcpStore', () => {
  let mcpStore: any;

  beforeEach(async () => {
    vi.resetModules();
    const mod = await import('../stores/mcp.svelte');
    mcpStore = mod.mcpStore;
  });

  it('loads servers, tools, and policy', async () => {
    await mcpStore.load();

    expect(mcpStore.servers).toHaveLength(2);
    expect(mcpStore.servers[0].name).toBe('builtin');

    expect(mcpStore.tools).toHaveLength(2);

    expect(mcpStore.policy.global_policy).toBe('allow');

    expect(mcpStore.loading).toBe(false);

    expect(mcpStore.error).toBeNull();
  });

  it('computes derived state', async () => {
    await mcpStore.load();

    const grouped = mcpStore.toolsByServer;
    expect(grouped['builtin']).toHaveLength(1);

    expect(mcpStore.pinWarningCount).toBe(1);

    expect(mcpStore.totalTools).toBe(2);

    expect(mcpStore.runningCount).toBe(1);
  });

  it('toggleServer calls API and reloads', async () => {
    await mcpStore.load();
    await mcpStore.toggleServer('builtin', false);
    const { setMcpServerEnabled } = await import('../api');
    expect(setMcpServerEnabled).toHaveBeenCalledWith('builtin', false);
  });

  it('addServer calls API and reloads', async () => {
    await mcpStore.load();
    await mcpStore.addServer('new-srv', 'http://new', { 'X-H': 'v' }, 'tok');
    const { addMcpServer } = await import('../api');
    expect(addMcpServer).toHaveBeenCalledWith('new-srv', 'http://new', { 'X-H': 'v' }, 'tok');
  });

  it('removeServer calls API and reloads', async () => {
    await mcpStore.load();
    await mcpStore.removeServer('external');
    const { removeMcpServer } = await import('../api');
    expect(removeMcpServer).toHaveBeenCalledWith('external');
  });

  it('setGlobalPolicy calls API and reloads', async () => {
    await mcpStore.load();
    await mcpStore.setGlobalPolicy('deny');
    const { setMcpGlobalPolicy } = await import('../api');
    expect(setMcpGlobalPolicy).toHaveBeenCalledWith('deny');
  });

  it('setDefaultPermission calls API and reloads', async () => {
    await mcpStore.load();
    await mcpStore.setDefaultPermission('warn');
    const { setMcpDefaultPermission } = await import('../api');
    expect(setMcpDefaultPermission).toHaveBeenCalledWith('warn');
  });

  it('setToolPermission calls API and reloads', async () => {
    await mcpStore.load();
    await mcpStore.setToolPermission('bash', 'block');
    const { setMcpToolPermission } = await import('../api');
    expect(setMcpToolPermission).toHaveBeenCalledWith('bash', 'block');
  });

  it('approveTool calls API and reloads', async () => {
    await mcpStore.load();
    await mcpStore.approveTool('bash');
    const { approveMcpTool } = await import('../api');
    expect(approveMcpTool).toHaveBeenCalledWith('bash');
  });

  it('refresh with server calls API', async () => {
    await mcpStore.load();
    await mcpStore.refresh('builtin');
    const { refreshMcpTools } = await import('../api');
    expect(refreshMcpTools).toHaveBeenCalledWith('builtin');
  });

  it('refresh without server calls API', async () => {
    await mcpStore.load();
    await mcpStore.refresh();
    const { refreshMcpTools } = await import('../api');
    expect(refreshMcpTools).toHaveBeenCalledWith(undefined);
  });

  it('handles load error', async () => {
    const { getMcpServers } = await import('../api');
    (getMcpServers as any).mockRejectedValueOnce(new Error('boom'));
    await mcpStore.load();
    expect(mcpStore.error).toContain('boom');
  });
});
