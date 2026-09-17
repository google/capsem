import {Client} from './client.js';
import {Debug} from './debug.js';
import {commandDeadlineMs} from './execution.js';
import * as api from './operations/index.js';
import * as models from './models/index.js';
import type {CreateOptions, HostLogOptions, RunOptions, Runtime, VmSelector} from './options.js';
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
  #defaults: Promise<models.ProfileDefaults> | undefined;
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
  /**
   * The profile the catalog uses for `runtime` when a call names none, read
   * from `GET /status` on first use and cached, so the SDK carries no profile
   * name. A container brings its own userland, so its default is the
   * catalog's to answer apart from a VM's.
   */
  async defaultProfileId(runtime: Runtime = 'vm', options: CallOptions = {}): Promise<string> {
    this.#defaults ??= this.info(options).then(info => info.profiles?.defaults ?? {}).catch((error: unknown) => {
      this.#defaults = undefined; // A failed lookup must not be cached.
      throw error;
    });
    const id = (await this.#defaults)[runtime];
    if (!id) {
      throw new TypeError(`The gateway profile catalog names no default ${runtime} profile; pass a profile from hypervisor.profiles.list()`);
    }
    return id;
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
    // Every local check first: an invalid argument must be refused before
    // the client asks the gateway anything.
    const ram_mb = memoryMb(options.memory);
    const container = options.image === undefined ? undefined : {
      image: options.image,
      args: [...(options.command ?? [])],
      env: options.env ?? {},
      ...(options.registry === undefined ? {} : {registry: {...options.registry}}),
      attach: false,
    };
    const profileId = options.profile?.id ?? await this.defaultProfileId(options.image === undefined ? 'vm' : 'container', options);
    const response = await api.createVm(this.transport, {body: {
      profile_id: profileId, name: options.name || null, persistent: Boolean(options.name),
      cpus: options.cpus ?? null, ram_mb,
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
    const ram_mb = memoryMb(options.memory);
    const profileId = options.profile?.id ?? await this.defaultProfileId('vm', options);
    return api.runVm(this.transport, {body: {
      command, profile_id: profileId, timeout_secs: options.timeout_secs ?? null,
      cpus: options.cpus ?? null, ram_mb, env: options.env ?? null,
    }}, {...options, timeoutMs: options.timeoutMs ?? commandDeadlineMs(this.transport.timeoutMs, options.timeout_secs)});
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
