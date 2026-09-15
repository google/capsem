import {Hypervisor} from '@capsem/sdk';
import {McpServer} from '@modelcontextprotocol/sdk/server/mcp.js';
import type {Config} from './config.js';
import {registerHostTools} from './host-tools.js';
import {toolCall} from './results.js';

export function createServer(config: Config): McpServer {
  // The SDK validates HTTP(S), bearer credentials and the transport deadline.
  // Tool closures added here share this one connection pool.
  const hypervisor = new Hypervisor(config.gatewayUrl, config.token, {timeoutMs: config.timeoutMs});
  const server = new McpServer({name: 'capsem-mcp', version: '0.6.3'});
  server.registerTool('capsem_status', {
    description: 'Read gateway, profile, update, and service status.',
  }, () => toolCall(() => hypervisor.info()));
  registerHostTools(server, hypervisor);
  return server;
}
