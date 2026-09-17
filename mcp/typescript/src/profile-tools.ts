import type {Hypervisor, Value} from '@capsem/sdk';
import type {McpServer} from '@modelcontextprotocol/sdk/server/mcp.js';
import {z} from 'zod';
import {toolCall} from './results.js';

const profileId = z.string().min(1).default('code');
const serverId = z.string().min(1);

export function registerProfileTools(server: McpServer, hypervisor: Hypervisor): void {
  server.registerTool('capsem_profiles', {
    description: 'List profiles and their availability through the gateway catalog.',
  }, () => toolCall(async () => ({profiles: await hypervisor.profiles.list()})));
  server.registerTool('capsem_mcp_info', {
    description: 'Read profile MCP configuration, readiness, and discovery summary.',
    inputSchema: {profile: profileId},
  }, ({profile}) => toolCall(() => hypervisor.profiles.mcp(profile).info()));
  server.registerTool('capsem_mcp_servers', {
    description: 'List configured MCP servers and their discovery status for a profile.',
    inputSchema: {profile: profileId},
  }, ({profile}) => toolCall(async () => ({servers: await hypervisor.profiles.mcp(profile).servers()})));
  server.registerTool('capsem_mcp_default', {
    description: 'Read the default MCP permission for a profile.',
    inputSchema: {profile: profileId},
  }, ({profile}) => toolCall(() => hypervisor.profiles.mcp(profile).defaultPermission()));
  server.registerTool('capsem_mcp_tools', {
    description: 'List discovered tools for one configured profile MCP server.',
    inputSchema: {profile: profileId, server_id: serverId},
  }, ({profile, server_id}) => toolCall(async () => ({
    tools: await hypervisor.profiles.mcp(profile).tools(server_id),
  })));
  server.registerTool('capsem_mcp_refresh', {
    description: 'Request fresh tool discovery for one configured profile MCP server.',
    inputSchema: {profile: profileId, server_id: serverId},
  }, ({profile, server_id}) => toolCall(() => hypervisor.profiles.mcp(profile).refresh(server_id)));
  server.registerTool('capsem_mcp_call', {
    description: 'Call one discovered profile MCP tool through gateway policy enforcement.',
    inputSchema: {
      profile: profileId, server_id: serverId, tool_id: z.string().min(1),
      arguments: z.json().default({}),
    },
  }, ({profile, server_id, tool_id, arguments: arguments_}) => toolCall(async () => ({
    result: await hypervisor.profiles.mcp(profile).call(server_id, tool_id, arguments_ as Value),
  })));
}
