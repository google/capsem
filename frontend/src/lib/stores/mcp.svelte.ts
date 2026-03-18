// MCP store -- loads servers, tools, and policy for the MCP settings section.
import {
  getMcpServers,
  getMcpTools,
  getMcpPolicy,
  setMcpServerEnabled,
  addMcpServer,
  removeMcpServer,
  setMcpGlobalPolicy,
  setMcpDefaultPermission,
  setMcpToolPermission,
  approveMcpTool,
  refreshMcpTools,
} from '../api';
import type { McpServerInfo, McpToolInfo, McpPolicyInfo } from '../types';

class McpStore {
  servers = $state<McpServerInfo[]>([]);
  tools = $state<McpToolInfo[]>([]);
  policy = $state<McpPolicyInfo>({
    global_policy: null,
    default_tool_permission: 'allow',
    blocked_servers: [],
    tool_permissions: {},
  });
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
  runningCount = $derived(this.servers.filter((s) => s.running).length);

  async load() {
    this.loading = true;
    this.error = null;
    try {
      const [servers, tools, policy] = await Promise.all([
        getMcpServers(),
        getMcpTools(),
        getMcpPolicy(),
      ]);
      this.servers = servers;
      this.tools = tools;
      this.policy = policy;
    } catch (e) {
      console.error('Failed to load MCP data:', e);
      this.error = String(e);
    } finally {
      this.loading = false;
    }
  }

  async toggleServer(name: string, enabled: boolean) {
    await setMcpServerEnabled(name, enabled);
    await this.load();
  }

  async addServer(name: string, url: string, headers: Record<string, string>, bearerToken: string | null) {
    await addMcpServer(name, url, headers, bearerToken);
    await this.load();
  }

  async removeServer(name: string) {
    await removeMcpServer(name);
    await this.load();
  }

  async setGlobalPolicy(policy: string) {
    await setMcpGlobalPolicy(policy);
    await this.load();
  }

  async setDefaultPermission(permission: string) {
    await setMcpDefaultPermission(permission);
    await this.load();
  }

  async setToolPermission(tool: string, permission: string) {
    await setMcpToolPermission(tool, permission);
    await this.load();
  }

  async approveTool(tool: string) {
    await approveMcpTool(tool);
    await this.load();
  }

  async refresh(server?: string) {
    await refreshMcpTools(server);
    await this.load();
  }
}

export const mcpStore = new McpStore();
