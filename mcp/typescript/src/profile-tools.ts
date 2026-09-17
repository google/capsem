import type {Hypervisor, Value} from '@capsem/sdk';
import type {McpServer} from '@modelcontextprotocol/sdk/server/mcp.js';
import {z} from 'zod';
import {toolCall} from './results.js';

/** Omitted means the catalog's default profile, which the gateway names. */
const profileId = z.string().min(1).optional();
const serverId = z.string().min(1);

async function profileMcp(hypervisor: Hypervisor, requested: string | undefined, signal: AbortSignal) {
  const id = requested ?? await hypervisor.defaultProfileId({signal});
  const matches = (await hypervisor.profiles.list({signal})).filter(profile => profile.id === id);
  const profile = matches[0];
  if (profile === undefined || matches.length !== 1) {
    throw new TypeError(`Expected one profile with ID ${JSON.stringify(id)}, found ${matches.length}`);
  }
  return hypervisor.profiles.mcp(profile);
}

export function registerProfileTools(server: McpServer, hypervisor: Hypervisor): void {
  server.registerTool('capsem_profiles', {
    description: 'List profiles and their availability through the gateway catalog.',
  }, extra => toolCall(async () => ({
    profiles: await hypervisor.profiles.list({signal: extra.signal}),
  })));
  server.registerTool('capsem_mcp_info', {
    description: 'Read profile MCP configuration, readiness, and discovery summary.',
    inputSchema: {profile: profileId},
  }, ({profile}, extra) => toolCall(async () =>
    (await profileMcp(hypervisor, profile, extra.signal)).info({signal: extra.signal})));
  server.registerTool('capsem_mcp_servers', {
    description: 'List configured MCP servers and their discovery status for a profile.',
    inputSchema: {profile: profileId},
  }, ({profile}, extra) => toolCall(async () => ({
    servers: await (await profileMcp(hypervisor, profile, extra.signal)).servers({signal: extra.signal}),
  })));
  server.registerTool('capsem_mcp_default', {
    description: 'Read the default MCP permission for a profile.',
    inputSchema: {profile: profileId},
  }, ({profile}, extra) => toolCall(async () =>
    (await profileMcp(hypervisor, profile, extra.signal)).defaultPermission({signal: extra.signal})));
  server.registerTool('capsem_mcp_tools', {
    description: 'List discovered tools for one configured profile MCP server.',
    inputSchema: {profile: profileId, server_id: serverId},
  }, ({profile, server_id}, extra) => toolCall(async () => {
    const options = {signal: extra.signal};
    const server = await (await profileMcp(hypervisor, profile, extra.signal)).get(server_id, options);
    return {tools: await server.tools.list(options)};
  }));
  server.registerTool('capsem_mcp_refresh', {
    description: 'Request fresh tool discovery for one configured profile MCP server.',
    inputSchema: {profile: profileId, server_id: serverId},
  }, ({profile, server_id}, extra) => toolCall(async () => {
    const options = {signal: extra.signal};
    return (await (await profileMcp(hypervisor, profile, extra.signal)).get(server_id, options)).refresh(options);
  }));
  server.registerTool('capsem_mcp_call', {
    description: 'Call one discovered profile MCP tool through gateway policy enforcement.',
    inputSchema: {
      profile: profileId, server_id: serverId, tool_id: z.string().min(1),
      arguments: z.json().default({}),
    },
  }, ({profile, server_id, tool_id, arguments: arguments_}, extra) => toolCall(async () => {
    const options = {signal: extra.signal};
    const server = await (await profileMcp(hypervisor, profile, extra.signal)).get(server_id, options);
    return {result: await server.tools.call(tool_id, arguments_ as Value, options)};
  }));
}
