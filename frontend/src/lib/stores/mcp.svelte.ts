// MCP store -- loads servers, tools, and policy for the MCP settings section.
import {
  getMcpServers,
  getMcpTools,
  setMcpServerEnabled,
  addMcpServer,
  removeMcpServer,
  approveMcpTool,
  refreshMcpTools,
} from '../api';
import type { McpServerInfo, McpToolInfo } from '../types';

class McpStore {
  servers = $state<McpServerInfo[]>([]);
  tools = $state<McpToolInfo[]>([]);
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
      const [servers, tools] = await Promise.all([
        getMcpServers(),
        getMcpTools(),
      ]);
      this.servers = servers;
      this.tools = tools;
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
