import {Hypervisor} from '@capsem/sdk';
import {McpServer} from '@modelcontextprotocol/sdk/server/mcp.js';
import type {Config} from './config.js';

export function createServer(config: Config): McpServer {
  // The SDK validates HTTP(S), bearer credentials and the transport deadline.
  // Tool closures added here share this one connection pool.
  const hypervisor = new Hypervisor(config.gatewayUrl, config.token, {timeoutMs: config.timeoutMs});
  const server = new McpServer({name: 'capsem-mcp', version: '0.6.3'});
  server.registerTool('capsem_status', {
    description: 'Read gateway, profile, update, and service status.',
  }, async () => {
    try {
      const status = await hypervisor.info();
      return {content: [{type: 'text', text: JSON.stringify(status)}], structuredContent: {...status}};
    } catch (error) {
      const message = error instanceof Error ? error.message : 'gateway request failed';
      return {content: [{type: 'text', text: message}], isError: true};
    }
  });
  return server;
}
