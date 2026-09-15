import {ExposureTarget, type Hypervisor} from '@capsem/sdk';
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

  server.registerTool('capsem_container_wait', {
    description: 'Wait by read-only polling until a VM container reaches a stable state; cancellation leaves the VM running.',
    inputSchema: {vm_id: vmId, interval_ms: z.number().int().positive().optional()},
  }, ({vm_id, interval_ms}, extra) => toolCall(() => hypervisor.vm({id: vm_id}).container.wait({
    ...(interval_ms === undefined ? {} : {intervalMs: interval_ms}), signal: extra.signal,
  })));

  server.registerTool('capsem_exposure_create', {
    description: 'Expose one VM or container port on host loopback through the authenticated, policy-checked lifecycle.',
    inputSchema: {
      vm_id: vmId,
      target: z.nativeEnum(ExposureTarget),
      guest_port: port,
      host_port: z.number().int().min(0).max(65_535).optional(),
    },
  }, ({vm_id, target, guest_port, host_port}, extra) => toolCall(() =>
    hypervisor.vm({id: vm_id}).exposures.create({
      target, guest_port, ...(host_port === undefined ? {} : {host_port}),
    }, {signal: extra.signal})));

  server.registerTool('capsem_exposure_list', {
    description: 'List the live loopback exposures owned by a VM process.',
    inputSchema: {vm_id: vmId},
  }, ({vm_id}, extra) => toolCall(() => hypervisor.vm({id: vm_id}).exposures.list({signal: extra.signal})));

  server.registerTool('capsem_exposure_delete', {
    description: 'Revoke a live VM exposure and close its listener.',
    inputSchema: {vm_id: vmId, exposure_id: z.string().min(1)},
  }, ({vm_id, exposure_id}, extra) => toolCall(() =>
    hypervisor.vm({id: vm_id}).exposures.delete(exposure_id, {signal: extra.signal})));
}
