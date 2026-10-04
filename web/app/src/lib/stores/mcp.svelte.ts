// MCP store -- loads configured MCP servers, tools, and permissions.
import {
  getMcpDefaultPermission,
  getMcpServers,
  getMcpTools,
  updateMcpDefaultPermission,
  updateMcpToolPermission,
  refreshMcpTools,
} from '../api';
import type { McpDefaultPermission, McpServerInfo, McpToolInfo, ToolPermission } from '../types';

class McpStore {
  servers = $state<McpServerInfo[]>([]);
  tools = $state<McpToolInfo[]>([]);
  defaultPermission = $state<McpDefaultPermission | null>(null);
  loading = $state(false);
  error = $state<string | null>(null);

  /** Tools grouped by server_name. */
  toolsByServer = $derived.by(() => {
    const map: Record<string, McpToolInfo[]> = {};
    for (const t of this.tools) {
      if (!map[t.server_name]) map[t.server_name] = [];
      map[t.server_name].push(t);
    }
    return map;
  });

  /** Number of tools with pin_changed === true. */
  pinWarningCount = $derived(this.tools.filter((t) => t.pin_changed).length);

  /** Total tool count. */
  totalTools = $derived(this.tools.length);

  /** Number of running servers. */
  runningCount = $derived(this.servers.filter((s) => s.source !== 'builtin' && s.running).length);

  async load() {
    this.loading = true;
    this.error = null;
    try {
      const [servers, defaultPermission] = await Promise.all([
        getMcpServers(),
        getMcpDefaultPermission(),
      ]);
      const toolLists = await Promise.all(
        servers.map((server) => getMcpTools(server.name)),
      );
      this.servers = servers;
      this.defaultPermission = defaultPermission;
      this.tools = toolLists.flat();
    } catch (e) {
      console.error('Failed to load MCP data:', e);
      this.error = String(e);
    } finally {
      this.loading = false;
    }
  }

  async setToolPermission(tool: McpToolInfo | string, action: ToolPermission) {
    const target = typeof tool === 'string'
      ? this.tools.find((candidate) => candidate.namespaced_name === tool || candidate.original_name === tool)
      : tool;
    if (!target) throw new Error(`MCP tool not loaded: ${tool}`);
    await updateMcpToolPermission(target.server_name, target.original_name, action);
    await this.load();
  }

  async setDefaultPermission(action: ToolPermission) {
    await updateMcpDefaultPermission(action);
    await this.load();
  }

  async refresh(server?: string) {
    const serverIds = server ? [server] : this.servers.map((entry) => entry.name);
    await Promise.all(serverIds.map((serverId) => refreshMcpTools(serverId)));
    await this.load();
  }
}

export const mcpStore = new McpStore();
