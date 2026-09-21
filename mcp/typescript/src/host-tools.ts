import {HostLogSource, TimelineLayer, type Hypervisor, type ProfileSummary, type VM} from '@capsem/sdk';
import type {McpServer} from '@modelcontextprotocol/sdk/server/mcp.js';
import {z} from 'zod';
import {toolCall} from './results.js';

const vmId = z.string().min(1).describe('Immutable VM ID returned by capsem_list or capsem_create');
const positiveInt = z.number().int().positive();
/** Largest file window one tool call returns; the transfer itself is whole-file. */
const MAX_READ_BYTES = 256 * 1024;
const GUEST_PATHS = 'Paths are the guest\'s: /root/x in a VM, /workspace/x in its container, '
  + 'or x relative to the workspace; other absolute paths are refused.';
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

async function profileOption(hypervisor: Hypervisor, profileId: string | undefined, signal: AbortSignal): Promise<{
  profile: ProfileSummary
} | undefined> {
  // No name means the catalog default, which the SDK resolves from the
  // gateway; a named profile is validated against the catalog, whatever it is.
  if (profileId === undefined) return undefined;
  const profile = (await hypervisor.profiles.list({signal})).find(candidate => candidate.id === profileId);
  if (profile === undefined) throw new TypeError(`Unknown profile ${JSON.stringify(profileId)}`);
  return {profile};
}

export function registerHostTools(server: McpServer, hypervisor: Hypervisor): void {
  server.registerTool('capsem_list', {
    description: 'List VMs with identity, lifecycle, resources, and telemetry.',
  }, extra => toolCall(() => hypervisor.list({signal: extra.signal})));

  server.registerTool('capsem_create', {
    description: 'Create a detached profile-owned VM and return its immutable ID.',
    inputSchema: {
      profile: z.string().min(1).optional(),
      name: z.string().min(1).optional(),
      cpus: positiveInt.optional(),
      memory: positiveInt.optional().describe('Guest memory in GiB'),
      env: z.record(z.string(), z.string()).optional(),
      network_ids: z.array(z.string().min(1)).optional(),
      image: z.string().min(1).optional(),
      command: z.array(z.string()).optional(),
      registry: registry.optional(),
    },
  }, ({profile, network_ids, registry: access, ...options}, extra) => toolCall(async () => {
    const networks = await Promise.all((network_ids ?? []).map(id =>
      hypervisor.networks.inspect(id, {signal: extra.signal})));
    const created = await hypervisor.create({
      ...defined(options), ...(await profileOption(hypervisor, profile, extra.signal) ?? {}), networks,
      ...(access === undefined ? {} : {registry: defined(access)}), signal: extra.signal,
    });
    return {id: created.id, name: created.name};
  }));

  server.registerTool('capsem_info', {
    description: 'Read lifecycle, resource, storage, network, and telemetry details for a VM.',
    inputSchema: {vm_id: vmId},
  }, ({vm_id}, extra) => toolCall(() => vm(hypervisor, vm_id).info({signal: extra.signal})));

  server.registerTool('capsem_exec', {
    description: 'Run a shell command in an existing VM and return stdout, stderr, and exit code.',
    inputSchema: {vm_id: vmId, command: z.string().min(1), timeout_secs: positiveInt.optional()},
  }, ({vm_id, command, timeout_secs}, extra) => toolCall(() =>
    vm(hypervisor, vm_id).exec(command, {...defined({timeout_secs}), signal: extra.signal})));

  server.registerTool('capsem_run', {
    description: 'Run a command in a fresh service-managed VM and return stdout, stderr, and exit code.',
    inputSchema: {
      command: z.string().min(1), profile: z.string().min(1).optional(), timeout_secs: positiveInt.optional(),
      cpus: positiveInt.optional(),
      memory: positiveInt.optional().describe('Guest memory in GiB'),
      env: z.record(z.string(), z.string()).optional(),
    },
  }, ({command, profile, ...options}, extra) => toolCall(async () => hypervisor.run(command, {
    ...defined(options), ...(await profileOption(hypervisor, profile, extra.signal) ?? {}), signal: extra.signal,
  })));

  server.registerTool('capsem_start', {
    description: 'Start a stopped VM.', inputSchema: {vm_id: vmId},
  }, ({vm_id}, extra) => toolCall(() => vm(hypervisor, vm_id).start({signal: extra.signal})));
  server.registerTool('capsem_stop', {
    description: 'Stop a VM.', inputSchema: {vm_id: vmId},
  }, ({vm_id}, extra) => toolCall(() => vm(hypervisor, vm_id).stop({signal: extra.signal})));
  server.registerTool('capsem_pause', {
    description: 'Pause a running VM.', inputSchema: {vm_id: vmId},
  }, ({vm_id}, extra) => toolCall(() => vm(hypervisor, vm_id).pause({signal: extra.signal})));
  server.registerTool('capsem_resume', {
    description: 'Resume a stopped or paused VM.', inputSchema: {vm_id: vmId},
  }, ({vm_id}, extra) => toolCall(() => vm(hypervisor, vm_id).resume({signal: extra.signal})));
  server.registerTool('capsem_delete', {
    description: 'Delete a VM and destroy its owned state.', inputSchema: {vm_id: vmId},
  }, ({vm_id}, extra) => toolCall(() => vm(hypervisor, vm_id).delete({signal: extra.signal})));

  server.registerTool('capsem_fork', {
    description: 'Fork a VM into a new stopped VM.',
    inputSchema: {vm_id: vmId, name: z.string().min(1), description: z.string().optional()},
  }, ({vm_id, name, description}, extra) => toolCall(async () => {
    const fork = await vm(hypervisor, vm_id).fork(name, {...defined({description}), signal: extra.signal});
    return {id: fork.id, name: fork.name};
  }));
  server.registerTool('capsem_persist', {
    description: 'Persist an ephemeral VM under a stable name.',
    inputSchema: {vm_id: vmId, name: z.string().min(1)},
  }, ({vm_id, name}, extra) => toolCall(() => vm(hypervisor, vm_id).persist(name, {signal: extra.signal})));
  server.registerTool('capsem_purge', {
    description: 'Purge VMs that the service reports as purgeable.',
    inputSchema: {all: z.boolean().optional()},
  }, (args, extra) => toolCall(() => hypervisor.purge({...defined(args), signal: extra.signal})));

  server.registerTool('capsem_list_files', {
    description: `List files in a VM using the gateway file API. ${GUEST_PATHS} An empty path lists the workspace root.`,
    inputSchema: {vm_id: vmId, path: z.string().default(''), depth: positiveInt.optional()},
  }, ({vm_id, path, depth}, extra) => toolCall(() =>
    vm(hypervisor, vm_id).files.list(path, {...defined({depth}), signal: extra.signal})));
  server.registerTool('capsem_read_file', {
    description: `Read a bounded window of a VM file as UTF-8 text or base64 through the gateway file API. ${GUEST_PATHS}`,
    inputSchema: {
      vm_id: vmId, path: z.string().min(1), encoding: z.enum(['utf8', 'base64']).default('utf8'),
      offset: z.number().int().nonnegative().default(0),
      max_bytes: positiveInt.max(MAX_READ_BYTES).default(MAX_READ_BYTES),
    },
  }, ({vm_id, path, encoding, offset, max_bytes}, extra) => toolCall(async () => {
    const data = await vm(hypervisor, vm_id).files.read(path, {signal: extra.signal});
    const window = Buffer.from(data).subarray(offset, offset + max_bytes);
    return {
      path, encoding, size: data.byteLength, offset,
      content: window.toString(encoding),
      truncated: offset + window.byteLength < data.byteLength,
    };
  }));
  server.registerTool('capsem_write_file', {
    description: `Write UTF-8 text or base64 bytes to a VM through the gateway file API. ${GUEST_PATHS} Returns the path the guest sees it at.`,
    inputSchema: {
      vm_id: vmId, path: z.string().min(1), content: z.string(), encoding: z.enum(['utf8', 'base64']).default('utf8'),
    },
  }, ({vm_id, path, content, encoding}, extra) => toolCall(() =>
    vm(hypervisor, vm_id).files.write(path, bytes(content, encoding), {signal: extra.signal})));

  server.registerTool('capsem_vm_logs', {
    description: 'Read serial and process logs for a VM.',
    inputSchema: {vm_id: vmId, ...logFilters},
  }, ({vm_id, ...options}, extra) => toolCall(() =>
    vm(hypervisor, vm_id).log({...defined(options), signal: extra.signal})));
  server.registerTool('capsem_host_logs', {
    description: 'Read an allowlisted host log through the authenticated gateway.',
    inputSchema: {source: z.nativeEnum(HostLogSource).default(HostLogSource.SERVICE), ...logFilters},
  }, (options, extra) => toolCall(() => hypervisor.log({...defined(options), signal: extra.signal})));
  server.registerTool('capsem_panics', {
    description: 'Read structured host panics, newest first.',
    inputSchema: {since: z.string().optional(), limit: positiveInt.optional()},
  }, (options, extra) => toolCall(() => hypervisor.debug.panics({...defined(options), signal: extra.signal})));
  server.registerTool('capsem_triage', {
    description: 'Correlate recent host failures, optionally with one VM\'s session.',
    inputSchema: {vm_id: vmId.optional(), since: z.string().optional(), limit: positiveInt.optional()},
  }, (options, extra) => toolCall(() => hypervisor.debug.triage({...defined(options), signal: extra.signal})));
  server.registerTool('capsem_timeline', {
    description: 'Read the correlated exec, tool, network, file, and model timeline for a VM.',
    inputSchema: {
      vm_id: vmId, trace_id: z.string().optional(), since: z.string().optional(), limit: positiveInt.optional(),
      layers: z.array(z.nativeEnum(TimelineLayer)).optional(),
    },
  }, ({vm_id, ...options}, extra) => toolCall(() =>
    vm(hypervisor, vm_id).timeline({...defined(options), signal: extra.signal})));
  server.registerTool('capsem_history', {
    description: 'Read paginated command and audit history for a VM.',
    inputSchema: {vm_id: vmId, ...page, search: z.string().optional()},
  }, ({vm_id, ...options}, extra) => toolCall(() =>
    vm(hypervisor, vm_id).history({...defined(options), signal: extra.signal})));
  server.registerTool('capsem_stats', {
    description: 'Read aggregate telemetry statistics for a VM.', inputSchema: {vm_id: vmId},
  }, ({vm_id}, extra) => toolCall(() => vm(hypervisor, vm_id).stats.summary({signal: extra.signal})));
  server.registerTool('capsem_stats_detail', {
    description: 'Read typed security, network, file, process, tool, and model events for a VM.',
    inputSchema: {vm_id: vmId},
  }, ({vm_id}, extra) => toolCall(() => vm(hypervisor, vm_id).stats.details({signal: extra.signal})));
  server.registerTool('capsem_snapshots', {
    description: 'List VM filesystem snapshots.', inputSchema: {vm_id: vmId},
  }, ({vm_id}, extra) => toolCall(() => vm(hypervisor, vm_id).snapshots.list({signal: extra.signal})));
  server.registerTool('capsem_snapshot_status', {
    description: 'Read VM filesystem snapshot readiness.', inputSchema: {vm_id: vmId},
  }, ({vm_id}, extra) => toolCall(() => vm(hypervisor, vm_id).snapshots.status({signal: extra.signal})));
  server.registerTool('capsem_file_history', {
    description: 'Read paginated filesystem changes since a snapshot checkpoint.',
    inputSchema: {vm_id: vmId, checkpoint: z.string().min(1), ...page},
  }, ({vm_id, checkpoint, ...options}, extra) => toolCall(() =>
    vm(hypervisor, vm_id).files.history(checkpoint, {...defined(options), signal: extra.signal})));
}
