import {Client} from './client.js';
import * as api from './operations/index.js';
import * as models from './models/index.js';
import type {CreateOptions, HostLogOptions} from './options.js';
import {Transport, type CallOptions, type TransportOptions} from './transport.js';
import {VM} from './vm.js';

function memoryMb(memory: string | number | undefined): number | null {
  if (memory === undefined) return null;
  if (typeof memory === 'string') {
    const match = /^([1-9][0-9]*)(M|G)$/i.exec(memory);
    if (!match) throw new TypeError("Memory must be a positive MB count or a size such as '512M' or '8G'");
    memory = Number(match[1]) * (match[2]?.toUpperCase() === 'G' ? 1024 : 1);
  }
  if (!Number.isSafeInteger(memory) || memory <= 0) throw new TypeError('Memory must be a positive MB count');
  return memory;
}

export class Hypervisor extends Client {
  constructor(url: string, token: string, options: TransportOptions = {}) {
    super(new Transport(url, token, options));
  }
  async info(options: CallOptions = {}): Promise<models.HypervisorInfo> {
    return api.getHypervisorInfo(this.transport, options);
  }
  async list(options: CallOptions = {}): Promise<models.ListResponse> {
    return api.listVms(this.transport, options);
  }
  async create(profile: string, options: CreateOptions = {}): Promise<VM> {
    if (options.vcpu !== undefined && (!Number.isSafeInteger(options.vcpu) || options.vcpu < 1)) {
      throw new TypeError('vcpu must be positive');
    }
    const response = await api.createVm(this.transport, {body: {
      profile_id: profile, name: options.name || null, persistent: Boolean(options.name),
      cpus: options.vcpu ?? null, ram_mb: memoryMb(options.memory), env: options.env ?? null,
    }}, options);
    return VM.bind(this.transport, response.id, response.name);
  }
  async log(options: HostLogOptions = {}): Promise<models.HostLogsResponse> {
    return api.getHypervisorLogs(this.transport, {...options, name: options.source ?? models.HostLogSource.SERVICE}, options);
  }
  async update(options: CallOptions = {}): Promise<models.UpdateActionResponse> {
    return api.updateHypervisor(this.transport, {body: {confirmed: true}}, options);
  }
  async restart(options: CallOptions = {}): Promise<models.RestartResponse> {
    return api.restartHypervisor(this.transport, options);
  }
}
