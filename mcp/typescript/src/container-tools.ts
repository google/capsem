import type {Hypervisor} from '@capsem/sdk';
import type {McpServer} from '@modelcontextprotocol/sdk/server/mcp.js';
import {z} from 'zod';
import {toolCall} from './results.js';

const vmId = z.string().min(1).describe('Immutable VM ID returned by capsem_list or capsem_create');
const port = z.number().int().min(1).max(65_535);

export function registerContainerTools(server: McpServer, hypervisor: Hypervisor): void {
  server.registerTool('capsem_container_status', {
    description: 'Read the current pull, staging, startup, running, exit, or failure state of a VM container.',
    inputSchema: {vm_id: vmId},
  }, ({vm_id}, extra) => toolCall(() => hypervisor.vm({id: vm_id}).container.status({signal: extra.signal})));

  server.registerTool('capsem_port_open', {
    description: 'Open a workload port; the SDK selects its VM or container namespace.',
    inputSchema: {
      vm_id: vmId,
      guest_port: port,
      host_port: z.number().int().min(0).max(65_535).optional(),
      authenticate: z.boolean().optional(),
    },
  }, ({vm_id, guest_port, host_port, authenticate}, extra) => toolCall(async () => {
    const opened = await hypervisor.vm({id: vm_id}).ports.open(guest_port, {
      ...(host_port === undefined ? {} : {host: host_port}),
      ...(authenticate === undefined ? {} : {authenticate}), signal: extra.signal,
    });
    // The SDK keeps the token out of the Port's enumerable fields so it is
    // never logged; the agent that asked for the port is the one who needs it.
    return opened.bootstrapToken === undefined ? opened : {...opened, bootstrapToken: opened.bootstrapToken};
  }));

  server.registerTool('capsem_port_list', {
    description: 'List the live ports owned by a workload.',
    inputSchema: {vm_id: vmId},
  }, ({vm_id}, extra) => toolCall(async () => ({
    ports: await hypervisor.vm({id: vm_id}).ports.list({signal: extra.signal}),
  })));

  server.registerTool('capsem_port_close', {
    description: 'Close a live workload port.',
    inputSchema: {vm_id: vmId, port_id: z.string().min(1)},
  }, ({vm_id, port_id}, extra) => toolCall(async () => {
    const ports = hypervisor.vm({id: vm_id}).ports;
    const opened = (await ports.list({signal: extra.signal})).find(candidate => candidate.id === port_id);
    if (opened === undefined) throw new TypeError(`No open port has ID ${port_id}`);
    return ports.close(opened, {signal: extra.signal});
  }));
}
