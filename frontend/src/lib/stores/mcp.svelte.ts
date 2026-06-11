// MCP store -- loads profile-owned MCP servers and tools.
import {
  getMcpServers,
  getMcpTools,
  updateMcpToolPermission,
  refreshMcpTools,
} from '../api';
import type { McpServerInfo, McpToolInfo, ToolPermission } from '../types';

class McpStore {
  servers = $state<McpServerInfo[]>([]);
  tools = $state<McpToolInfo[]>([]);
  loading = $state(false);
  error = $state<string | null>(null);
  profileId = $state<string | null>(null);

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
  runningCount = $derived(this.servers.filter((s) => s.running).length);

  private activeProfileId(): string {
    if (!this.profileId) throw new Error('MCP profile id is not loaded');
    return this.profileId;
  }

  async load(profileId: string) {
    this.profileId = profileId;
    this.loading = true;
    this.error = null;
    try {
      const servers = await getMcpServers(profileId);
      const toolLists = await Promise.all(
        servers.map((server) => getMcpTools(profileId, server.name)),
      );
      this.servers = servers;
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
    const profileId = this.activeProfileId();
    await updateMcpToolPermission(profileId, target.server_name, target.original_name, action);
    await this.load(profileId);
  }

  async refresh(server?: string) {
    const profileId = this.activeProfileId();
    const serverIds = server ? [server] : this.servers.map((entry) => entry.name);
    await Promise.all(serverIds.map((serverId) => refreshMcpTools(profileId, serverId)));
    await this.load(profileId);
  }
}

export const mcpStore = new McpStore();
