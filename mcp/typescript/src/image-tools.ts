import type {Hypervisor} from '@capsem/sdk';
import type {McpServer} from '@modelcontextprotocol/sdk/server/mcp.js';
import {z} from 'zod';
import {toolCall} from './results.js';

export function registerImageTools(server: McpServer, hypervisor: Hypervisor): void {
  server.registerTool('capsem_image_list', {
    description: 'Read the service-admitted image catalog, compatible immutable pins and reported cache state. Unknown cache state does not prove readiness.',
    inputSchema: z.strictObject({refresh: z.boolean().optional()}),
  }, ({refresh}, extra) => toolCall(() => hypervisor.images.list({refresh: refresh ?? false, signal: extra.signal})));

  server.registerTool('capsem_image_pull', {
    description: 'Prefetch an image through service admission and return its resolved pin. Does not create a VM; private access remains service-owned.',
    inputSchema: z.strictObject({image: z.string().min(1).refine(value => value.trim().length > 0, 'Image must be nonempty')}),
  }, ({image}, extra) => toolCall(() => hypervisor.images.pull(image, {signal: extra.signal})));
}
