import {createRequire} from 'node:module';
import {delegatedBearer, Hypervisor} from '@capsem/sdk';
import {McpServer} from '@modelcontextprotocol/sdk/server/mcp.js';
import type {Config} from './config.js';
import {registerContainerTools} from './container-tools.js';
import {registerHostTools} from './host-tools.js';
import {registerImageTools} from './image-tools.js';
import {registerMcpTools} from './mcp-tools.js';
import {registerNetworkTools} from './network-tools.js';
import {toolCall} from './results.js';

const manifest = createRequire(import.meta.url)('../package.json') as {version: string};

export function createServer(config: Config): McpServer {
  if (typeof config.gatewayUrl !== 'string' || !config.gatewayUrl) throw new TypeError('explicit gateway URL is required');
  // This boundary also serves programmatic callers: never accept a naked
  // operator token or let an absent URL select SDK local discovery.
  const token = delegatedBearer(config.accessId, config.capability);
  const hypervisor = new Hypervisor(config.gatewayUrl, token, {timeoutMs: config.timeoutMs});
  const server = new McpServer({name: 'capsem-mcp', version: manifest.version});
  server.registerTool('capsem_status', {
    description: 'Read gateway, VM asset, update, and service status.',
  }, extra => toolCall(() => hypervisor.info({signal: extra.signal})));
  registerHostTools(server, hypervisor);
  registerContainerTools(server, hypervisor);
  registerImageTools(server, hypervisor);
  registerNetworkTools(server, hypervisor);
  registerMcpTools(server, hypervisor);
  return server;
}
