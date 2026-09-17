import {NetworkDecision, type Hypervisor} from '@capsem/sdk';
import type {McpServer} from '@modelcontextprotocol/sdk/server/mcp.js';
import {z} from 'zod';
import {toolCall} from './results.js';

const networkId = z.string().min(1).describe('Immutable network ID');
const vmId = z.string().min(1).describe('Immutable VM ID');
const positiveInt = z.number().int().positive();
const unixMs = z.number().int().nonnegative();

function defined<T extends object>(input: T): {[K in keyof T]?: Exclude<T[K], undefined>} {
  return Object.fromEntries(Object.entries(input).filter(([, value]) => value !== undefined)) as {
    [K in keyof T]?: Exclude<T[K], undefined>
  };
}

export function registerNetworkTools(server: McpServer, hypervisor: Hypervisor): void {
  server.registerTool('capsem_network_create', {
    description: 'Create a private VM network and return its immutable ID and assigned subnet.',
    inputSchema: {name: z.string().min(1)},
  }, ({name}) => toolCall(() => hypervisor.networks.create(name)));
  server.registerTool('capsem_network_list', {
    description: 'List private VM networks and their current members.',
  }, () => toolCall(async () => ({networks: await hypervisor.networks.list()})));
  server.registerTool('capsem_network_inspect', {
    description: 'Inspect a private network by immutable ID.', inputSchema: {network_id: networkId},
  }, ({network_id}) => toolCall(() => hypervisor.networks.inspect(network_id)));
  server.registerTool('capsem_network_delete', {
    description: 'Retire a private network by immutable ID.', inputSchema: {network_id: networkId},
  }, ({network_id}) => toolCall(async () => {
    const network = await hypervisor.networks.inspect(network_id);
    return hypervisor.networks.delete(network);
  }));
  server.registerTool('capsem_network_attach', {
    description: 'Attach a VM to a private network and return actual membership state.',
    inputSchema: {network_id: networkId, vm_id: vmId},
  }, ({network_id, vm_id}) => toolCall(async () => {
    const network = await hypervisor.networks.inspect(network_id);
    return hypervisor.vm({id: vm_id}).networks.attach(network);
  }));
  server.registerTool('capsem_network_detach', {
    description: 'Detach a VM from a private network and return actual membership state.',
    inputSchema: {network_id: networkId, vm_id: vmId},
  }, ({network_id, vm_id}) => toolCall(async () => {
    const network = await hypervisor.networks.inspect(network_id);
    return hypervisor.vm({id: vm_id}).networks.detach(network);
  }));
  server.registerTool('capsem_network_logs', {
    description: 'Read a cursor-based private-network audit stream with optional correlation filters.',
    inputSchema: {
      network_id: networkId,
      cursor: z.string().optional(),
      limit: positiveInt.optional(),
      vm_id: vmId.optional(),
      connection_id: z.string().min(1).optional(),
      event_type: z.string().min(1).optional(),
      decision: z.nativeEnum(NetworkDecision).optional(),
      since_unix_ms: unixMs.optional(),
      until_unix_ms: unixMs.optional(),
    },
  }, ({network_id, vm_id, connection_id, event_type, since_unix_ms, until_unix_ms, ...options}) => toolCall(async () => {
    const network = await hypervisor.networks.inspect(network_id);
    return hypervisor.networks.logs(network, defined({
      ...options, vm: vm_id, connection: connection_id, type: event_type,
      since: since_unix_ms, until: until_unix_ms,
    }));
  }));
}
