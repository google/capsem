import type {Hypervisor, Value} from '@capsem/sdk';
import type {McpServer} from '@modelcontextprotocol/sdk/server/mcp.js';
import {z} from 'zod';
import {toolCall} from './results.js';

const profileId = z.string().min(1).default('code');
const serverId = z.string().min(1);

async function profileMcp(hypervisor: Hypervisor, id: string) {
  const matches = (await hypervisor.profiles.list()).filter(profile => profile.id === id);
  const profile = matches[0];
  if (profile === undefined || matches.length !== 1) {
    throw new TypeError(`Expected one profile with ID ${JSON.stringify(id)}, found ${matches.length}`);
  }
  return hypervisor.profiles.mcp(profile);
}

export function registerProfileTools(server: McpServer, hypervisor: Hypervisor): void {
  server.registerTool('capsem_profiles', {
    description: 'List profiles and their availability through the gateway catalog.',
  }, () => toolCall(async () => ({profiles: await hypervisor.profiles.list()})));
  server.registerTool('capsem_mcp_info', {
    description: 'Read profile MCP configuration, readiness, and discovery summary.',
    inputSchema: {profile: profileId},
  }, ({profile}) => toolCall(async () => (await profileMcp(hypervisor, profile)).info()));
  server.registerTool('capsem_mcp_servers', {
    description: 'List configured MCP servers and their discovery status for a profile.',
    inputSchema: {profile: profileId},
  }, ({profile}) => toolCall(async () => ({servers: await (await profileMcp(hypervisor, profile)).servers()})));
  server.registerTool('capsem_mcp_default', {
    description: 'Read the default MCP permission for a profile.',
    inputSchema: {profile: profileId},
  }, ({profile}) => toolCall(async () => (await profileMcp(hypervisor, profile)).defaultPermission()));
  server.registerTool('capsem_mcp_tools', {
    description: 'List discovered tools for one configured profile MCP server.',
    inputSchema: {profile: profileId, server_id: serverId},
  }, ({profile, server_id}) => toolCall(async () => {
    const server = await (await profileMcp(hypervisor, profile)).get(server_id);
    return {tools: await server.tools.list()};
  }));
  server.registerTool('capsem_mcp_refresh', {
    description: 'Request fresh tool discovery for one configured profile MCP server.',
    inputSchema: {profile: profileId, server_id: serverId},
  }, ({profile, server_id}) => toolCall(async () =>
    (await (await profileMcp(hypervisor, profile)).get(server_id)).refresh()));
  server.registerTool('capsem_mcp_call', {
    description: 'Call one discovered profile MCP tool through gateway policy enforcement.',
    inputSchema: {
      profile: profileId, server_id: serverId, tool_id: z.string().min(1),
      arguments: z.json().default({}),
    },
  }, ({profile, server_id, tool_id, arguments: arguments_}) => toolCall(async () => {
    const server = await (await profileMcp(hypervisor, profile)).get(server_id);
    return {result: await server.tools.call(tool_id, arguments_ as Value)};
  }));
}
