import {HostLogSource, TimelineLayer, type ContainerOptions, type Hypervisor, type VM} from '@capsem/sdk';
import type {McpServer} from '@modelcontextprotocol/sdk/server/mcp.js';
import {z} from 'zod';
import {toolCall} from './results.js';

const vmId = z.string().min(1).describe('Immutable VM ID returned by capsem_list or capsem_create');
const positiveInt = z.number().int().positive();
const page = {
  limit: positiveInt.optional(),
  offset: z.number().int().nonnegative().optional(),
};
const logFilters = {
  grep: z.string().optional(),
  tail: positiveInt.optional(),
  max_bytes: positiveInt.optional(),
};
const registry = z.object({
  username: z.string().optional(), password: z.string().optional(), ca_pem: z.string().optional(),
});
const container = z.object({
  image: z.string().min(1),
  args: z.array(z.string()).optional(),
  registry: registry.optional(),
  attach: z.boolean().optional(),
}).strict();

function vm(hypervisor: Hypervisor, id: string): VM {
  return hypervisor.vm({id});
}

function bytes(content: string, encoding: 'utf8' | 'base64'): Uint8Array {
  return encoding === 'base64' ? Uint8Array.from(Buffer.from(content, 'base64')) : new TextEncoder().encode(content);
}

function defined<T extends object>(input: T): {[K in keyof T]?: Exclude<T[K], undefined>} {
  return Object.fromEntries(Object.entries(input).filter(([, value]) => value !== undefined)) as {
    [K in keyof T]?: Exclude<T[K], undefined>
  };
}

function containerOptions(input: z.infer<typeof container>): ContainerOptions {
  const {image, registry: access, ...options} = input;
  return {
    image,
    ...defined(options),
    ...(access === undefined ? {} : {registry: defined(access)}),
  };
}

export function registerHostTools(server: McpServer, hypervisor: Hypervisor): void {
  server.registerTool('capsem_list', {
    description: 'List VMs with identity, lifecycle, resources, and telemetry.',
  }, () => toolCall(() => hypervisor.list()));

  server.registerTool('capsem_create', {
    description: 'Create a detached profile-owned VM and return its immutable ID.',
    inputSchema: {
      profile: z.string().min(1).default('code'),
      name: z.string().min(1).optional(),
      vcpu: positiveInt.optional(),
      memory: z.union([positiveInt, z.string().regex(/^[1-9][0-9]*[MG]$/i)]).optional(),
      env: z.record(z.string(), z.string()).optional(),
      networks: z.array(z.string().min(1)).optional(),
      container: container.optional(),
    },
  }, ({profile, container: workload, ...options}) => toolCall(async () => {
    const created = await hypervisor.create(profile, {
      ...defined(options),
      ...(workload === undefined ? {} : {container: containerOptions(workload)}),
    });
    return {id: created.id, name: created.name};
  }));

  server.registerTool('capsem_info', {
    description: 'Read lifecycle, resource, storage, network, and telemetry details for a VM.',
    inputSchema: {vm_id: vmId},
  }, ({vm_id}) => toolCall(() => vm(hypervisor, vm_id).info()));

  server.registerTool('capsem_exec', {
    description: 'Run a shell command in an existing VM and return stdout, stderr, and exit code.',
    inputSchema: {vm_id: vmId, command: z.string().min(1), timeout_secs: positiveInt.optional()},
  }, ({vm_id, command, timeout_secs}) => toolCall(() => vm(hypervisor, vm_id).exec(command, defined({timeout_secs}))));

  server.registerTool('capsem_run', {
    description: 'Run a command in a fresh service-managed VM and return stdout, stderr, and exit code.',
    inputSchema: {
      command: z.string().min(1), profile: z.string().min(1).optional(), timeout_secs: positiveInt.optional(),
      vcpu: positiveInt.optional(),
      memory: z.union([positiveInt, z.string().regex(/^[1-9][0-9]*[MG]$/i)]).optional(),
      env: z.record(z.string(), z.string()).optional(),
    },
  }, ({command, ...options}) => toolCall(() => hypervisor.run(command, defined(options))));

  server.registerTool('capsem_start', {
    description: 'Start a stopped VM.', inputSchema: {vm_id: vmId},
  }, ({vm_id}) => toolCall(() => vm(hypervisor, vm_id).start()));
  server.registerTool('capsem_stop', {
    description: 'Stop a VM.', inputSchema: {vm_id: vmId},
  }, ({vm_id}) => toolCall(() => vm(hypervisor, vm_id).stop()));
  server.registerTool('capsem_pause', {
    description: 'Pause a running VM.', inputSchema: {vm_id: vmId},
  }, ({vm_id}) => toolCall(() => vm(hypervisor, vm_id).pause()));
  server.registerTool('capsem_resume', {
    description: 'Resume a stopped or paused VM.', inputSchema: {vm_id: vmId},
  }, ({vm_id}) => toolCall(() => vm(hypervisor, vm_id).resume()));
  server.registerTool('capsem_delete', {
    description: 'Delete a VM and destroy its owned state.', inputSchema: {vm_id: vmId},
  }, ({vm_id}) => toolCall(() => vm(hypervisor, vm_id).delete()));

  server.registerTool('capsem_fork', {
    description: 'Fork a VM into a new stopped VM.',
    inputSchema: {vm_id: vmId, name: z.string().min(1), description: z.string().optional()},
  }, ({vm_id, name, description}) => toolCall(async () => {
    const fork = await vm(hypervisor, vm_id).fork(name, defined({description}));
    return {id: fork.id, name: fork.name};
  }));
  server.registerTool('capsem_persist', {
    description: 'Persist an ephemeral VM under a stable name.',
    inputSchema: {vm_id: vmId, name: z.string().min(1)},
  }, ({vm_id, name}) => toolCall(() => vm(hypervisor, vm_id).persist(name)));
  server.registerTool('capsem_purge', {
    description: 'Purge VMs that the service reports as purgeable.',
    inputSchema: {all: z.boolean().optional()},
  }, args => toolCall(() => hypervisor.purge(defined(args))));

  server.registerTool('capsem_list_files', {
    description: 'List files in a VM using the gateway file API.',
    inputSchema: {vm_id: vmId, path: z.string().default('/'), depth: positiveInt.optional()},
  }, ({vm_id, path, depth}) => toolCall(() => vm(hypervisor, vm_id).list(path, defined({depth}))));
  server.registerTool('capsem_read_file', {
    description: 'Read a VM file as UTF-8 text or base64 through the gateway file API.',
    inputSchema: {vm_id: vmId, path: z.string().min(1), encoding: z.enum(['utf8', 'base64']).default('utf8')},
  }, ({vm_id, path, encoding}) => toolCall(async () => {
    const data = await vm(hypervisor, vm_id).copy.fromVm(path);
    return {path, encoding, size: data.byteLength, content: Buffer.from(data).toString(encoding)};
  }));
  server.registerTool('capsem_write_file', {
    description: 'Write UTF-8 text or base64 bytes to a VM through the gateway file API.',
    inputSchema: {
      vm_id: vmId, path: z.string().min(1), content: z.string(), encoding: z.enum(['utf8', 'base64']).default('utf8'),
    },
  }, ({vm_id, path, content, encoding}) => toolCall(() => vm(hypervisor, vm_id).copy.toVm(path, bytes(content, encoding))));

  server.registerTool('capsem_vm_logs', {
    description: 'Read serial and process logs for a VM.',
    inputSchema: {vm_id: vmId, ...logFilters},
  }, ({vm_id, ...options}) => toolCall(() => vm(hypervisor, vm_id).log(defined(options))));
  server.registerTool('capsem_host_logs', {
    description: 'Read an allowlisted host log through the authenticated gateway.',
    inputSchema: {source: z.nativeEnum(HostLogSource).default(HostLogSource.SERVICE), ...logFilters},
  }, options => toolCall(() => hypervisor.log(defined(options))));
  server.registerTool('capsem_panics', {
    description: 'Read structured recent host panics before widening an investigation.',
    inputSchema: {since: z.string().optional(), limit: positiveInt.optional()},
  }, options => toolCall(() => hypervisor.panics(defined(options))));
  server.registerTool('capsem_triage', {
    description: 'Read ranked host diagnostics and optional VM ledger correlation.',
    inputSchema: {since: z.string().optional(), limit: positiveInt.optional(), vm_id: vmId.optional()},
  }, options => toolCall(() => hypervisor.triage(defined(options))));
  server.registerTool('capsem_timeline', {
    description: 'Read the correlated exec, tool, network, file, and model timeline for a VM.',
    inputSchema: {
      vm_id: vmId, trace_id: z.string().optional(), since: z.string().optional(), limit: positiveInt.optional(),
      layers: z.array(z.nativeEnum(TimelineLayer)).optional(),
    },
  }, ({vm_id, ...options}) => toolCall(() => vm(hypervisor, vm_id).timeline(defined(options))));
  server.registerTool('capsem_history', {
    description: 'Read paginated command and audit history for a VM.',
    inputSchema: {vm_id: vmId, ...page, search: z.string().optional()},
  }, ({vm_id, ...options}) => toolCall(() => vm(hypervisor, vm_id).history(defined(options))));
  server.registerTool('capsem_stats', {
    description: 'Read aggregate telemetry statistics for a VM.', inputSchema: {vm_id: vmId},
  }, ({vm_id}) => toolCall(() => vm(hypervisor, vm_id).stats.summary()));
  server.registerTool('capsem_stats_detail', {
    description: 'Read typed security, network, file, process, tool, and model events for a VM.',
    inputSchema: {vm_id: vmId},
  }, ({vm_id}) => toolCall(() => vm(hypervisor, vm_id).stats.details()));
  server.registerTool('capsem_snapshots', {
    description: 'List VM filesystem snapshots.', inputSchema: {vm_id: vmId},
  }, ({vm_id}) => toolCall(() => vm(hypervisor, vm_id).snapshots.list()));
  server.registerTool('capsem_snapshot_status', {
    description: 'Read VM filesystem snapshot readiness.', inputSchema: {vm_id: vmId},
  }, ({vm_id}) => toolCall(() => vm(hypervisor, vm_id).snapshots.status()));
  server.registerTool('capsem_changes', {
    description: 'Read paginated filesystem changes since a snapshot checkpoint.',
    inputSchema: {vm_id: vmId, checkpoint: z.string().min(1), ...page},
  }, ({vm_id, checkpoint, ...options}) => toolCall(() => vm(hypervisor, vm_id).changes(checkpoint, defined(options))));
}
