import type {Hypervisor, Value} from '@capsem/sdk';
import type {McpServer} from '@modelcontextprotocol/sdk/server/mcp.js';
import {z} from 'zod';
import {toolCall} from './results.js';

const serverId = z.string().min(1);

/** The MCP servers every VM runs: settings.toml `[mcp]` with corp's laid over it. */
export function registerMcpTools(server: McpServer, hypervisor: Hypervisor): void {
  server.registerTool('capsem_mcp_info', {
    description: 'Count the MCP servers every VM runs and whether the built-in local server is enabled.',
  }, extra => toolCall(() => hypervisor.mcp.info({signal: extra.signal})));
  server.registerTool('capsem_mcp_servers', {
    description: 'List the configured MCP servers every VM runs and their discovery status.',
  }, extra => toolCall(async () => ({servers: await hypervisor.mcp.servers({signal: extra.signal})})));
  server.registerTool('capsem_mcp_default', {
    description: 'Read the default MCP tool permission.',
  }, extra => toolCall(() => hypervisor.mcp.defaultPermission({signal: extra.signal})));
  server.registerTool('capsem_mcp_tools', {
    description: 'List discovered tools for one configured MCP server.',
    inputSchema: {server_id: serverId},
  }, ({server_id}, extra) => toolCall(async () => {
    const options = {signal: extra.signal};
    return {tools: await (await hypervisor.mcp.get(server_id, options)).tools.list(options)};
  }));
  server.registerTool('capsem_mcp_refresh', {
    description: 'Request fresh tool discovery for one configured MCP server.',
    inputSchema: {server_id: serverId},
  }, ({server_id}, extra) => toolCall(async () => {
    const options = {signal: extra.signal};
    return (await hypervisor.mcp.get(server_id, options)).refresh(options);
  }));
  server.registerTool('capsem_mcp_call', {
    description: 'Call one discovered MCP tool through gateway policy enforcement.',
    inputSchema: {server_id: serverId, tool_id: z.string().min(1), arguments: z.json().default({})},
  }, ({server_id, tool_id, arguments: arguments_}, extra) => toolCall(async () => {
    const options = {signal: extra.signal};
    const server = await hypervisor.mcp.get(server_id, options);
    return {result: await server.tools.call(tool_id, arguments_ as Value, options)};
  }));
}
