import {Client} from './client.js';
import {Debug} from './debug.js';
import * as api from './operations/index.js';
import * as models from './models/index.js';
import type {CreateOptions, HostLogOptions, RunOptions, VmSelector} from './options.js';
import {Transport, type CallOptions, type TransportOptions} from './transport.js';
import {Networks, Profiles} from './resources.js';
import {VM} from './vm.js';

function memoryMb(memory: number | undefined): number | null {
  if (memory === undefined) return null;
  if (!Number.isSafeInteger(memory) || memory <= 0) throw new TypeError('Memory must be a positive GiB count');
  const megabytes = memory * 1024;
  if (!Number.isSafeInteger(megabytes)) throw new TypeError('Memory is too large');
  return megabytes;
}

export class Hypervisor extends Client {
  readonly networks: Networks;
  readonly profiles: Profiles;
  readonly debug: Debug;
  constructor(url: string, token: string, options: TransportOptions = {}) {
    const transport = new Transport(url, token, options);
    super(transport);
    this.networks = new Networks(transport);
    this.profiles = new Profiles(transport);
    this.debug = new Debug(transport);
  }
  async info(options: CallOptions = {}): Promise<models.HypervisorInfo> {
    return api.getHypervisorInfo(this.transport, options);
  }
  async list(options: CallOptions = {}): Promise<models.ListResponse> {
    return api.listVms(this.transport, options);
  }
  vm(selector: VmSelector): VM {
    return new VM(this.transport, selector);
  }
  async create(options: CreateOptions = {}): Promise<VM> {
    if (options.cpus !== undefined && (!Number.isSafeInteger(options.cpus) || options.cpus < 1)) {
      throw new TypeError('cpus must be positive');
    }
    if (options.image === undefined && (options.command !== undefined || options.registry !== undefined)) {
      throw new TypeError('Container command and registry require an image');
    }
    if (options.image !== undefined && (typeof options.image !== 'string' || !options.image)) {
      throw new TypeError('Image must be a nonempty string');
    }
    const container = options.image === undefined ? undefined : {
      image: options.image,
      args: [...(options.command ?? [])],
      env: options.env ?? {},
      ...(options.registry === undefined ? {} : {registry: {...options.registry}}),
      attach: false,
    };
    const response = await api.createVm(this.transport, {body: {
      profile_id: options.profile?.id ?? 'code', name: options.name || null, persistent: Boolean(options.name),
      cpus: options.cpus ?? null, ram_mb: memoryMb(options.memory),
      env: container === undefined ? options.env ?? null : null,
      networks: (options.networks ?? []).map(network => network.name),
      ...(container === undefined ? {} : {container}),
    }}, options);
    return VM.bind(this.transport, response.id, response.name, container !== undefined);
  }
  async log(options: HostLogOptions = {}): Promise<models.HostLogsResponse> {
    return api.getHypervisorLogs(this.transport, {...options, name: options.source ?? models.HostLogSource.SERVICE}, options);
  }
  async run(command: string, options: RunOptions = {}): Promise<models.ExecResponse> {
    return api.runVm(this.transport, {body: {
      command, profile_id: options.profile?.id ?? 'code', timeout_secs: options.timeout_secs ?? null,
      cpus: options.cpus ?? null, ram_mb: memoryMb(options.memory), env: options.env ?? null,
    }}, options);
  }
  async purge(options: CallOptions & {all?: boolean} = {}): Promise<models.PurgeResponse> {
    return api.purgeVms(this.transport, {body: {all: options.all ?? false}}, options);
  }
  async update(options: CallOptions = {}): Promise<models.UpdateActionResponse> {
    return api.updateHypervisor(this.transport, {body: {confirmed: true}}, options);
  }
  async restart(options: CallOptions = {}): Promise<models.RestartResponse> {
    return api.restartHypervisor(this.transport, options);
  }
}
