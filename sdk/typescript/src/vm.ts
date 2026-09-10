import {Client} from './client.js';
import * as api from './operations/index.js';
import type * as models from './models/index.js';
import type {HistoryOptions, LogOptions, PageOptions, TimelineOptions, VmSelector} from './options.js';
import {Copy, Snapshots, Stats, type VmContext} from './resources.js';
import {Transport, type CallOptions, type TransportOptions} from './transport.js';

export class VM extends Client {
  #id: string | undefined;
  #name: string | undefined;
  readonly copy: Copy;
  readonly snapshots: Snapshots;
  readonly stats: Stats;

  constructor(url: string, token: string, selector: VmSelector, options?: TransportOptions);
  /** @internal */
  constructor(transport: Transport, selector: VmSelector);
  constructor(url: string | Transport, token: string | VmSelector, selector?: VmSelector, options: TransportOptions = {}) {
    super(typeof url === 'string' ? new Transport(url, typeof token === 'string' ? token : '', options) : url, typeof url === 'string');
    const selected = typeof token === 'string' ? selector : token;
    const id = selected?.id, name = selected?.name;
    if ((id === undefined) === (name === undefined)
      || (id !== undefined && (typeof id !== 'string' || !id))
      || (name !== undefined && (typeof name !== 'string' || !name))) {
      throw new TypeError('Select a VM by exactly one nonempty name or id');
    }
    this.#id = id;
    this.#name = name;
    const context = (call: CallOptions): Promise<VmContext> => this.context(call);
    this.copy = new Copy(context);
    this.snapshots = new Snapshots(context);
    this.stats = new Stats(context);
  }
  /** @internal */
  static bind(transport: Transport, id: string, name: string): VM {
    const vm = new VM(transport, {id});
    vm.#name = name;
    return vm;
  }
  get id(): string | undefined {return this.#id;}
  get name(): string | undefined {return this.#name;}

  private async context(options: CallOptions): Promise<VmContext> {
    const transport = this.transport;
    if (this.#id === undefined) {
      const inventory = await api.listVms(transport, options);
      const matches = inventory.sandboxes.filter(vm => vm.name === this.#name);
      const match = matches[0];
      if (matches.length !== 1 || !match) throw new Error(`Expected one VM named ${String(this.#name)}, found ${matches.length}`);
      this.#id = match.id;
    }
    return {transport, id: this.#id};
  }
  async info(options: CallOptions = {}): Promise<models.SandboxInfo> {
    const {transport, id} = await this.context(options);
    return api.getVmInfo(transport, {id}, options);
  }
  async exec(command: string, options: CallOptions & {timeout_secs?: number} = {}): Promise<models.ExecResponse> {
    const {transport, id} = await this.context(options);
    return api.execVm(transport, {id, body: {command, timeout_secs: options.timeout_secs ?? null}}, options);
  }
  async start(options: CallOptions = {}): Promise<models.ProvisionResponse> {
    const {transport, id} = await this.context(options);
    return api.startVm(transport, {id}, options);
  }
  async stop(options: CallOptions = {}): Promise<models.StopResponse> {
    const {transport, id} = await this.context(options);
    return api.stopVm(transport, {id}, options);
  }
  async pause(options: CallOptions = {}): Promise<models.VmActionResponse> {
    const {transport, id} = await this.context(options);
    return api.pauseVm(transport, {id}, options);
  }
  async resume(options: CallOptions = {}): Promise<models.ProvisionResponse> {
    const {transport, id} = await this.context(options);
    return api.resumeVm(transport, {id}, options);
  }
  async delete(options: CallOptions = {}): Promise<models.VmActionResponse> {
    const {transport, id} = await this.context(options);
    return api.deleteVm(transport, {id}, options);
  }
  async fork(name: string, options: CallOptions & {description?: string} = {}): Promise<VM> {
    const {transport, id} = await this.context(options);
    const response = await api.forkVm(transport, {id, body: {name, description: options.description ?? null}}, options);
    return VM.bind(transport, response.id, response.name);
  }
  async log(options: LogOptions = {}): Promise<models.LogsResponse> {
    const {transport, id} = await this.context(options);
    return api.getVmLogs(transport, {...options, id}, options);
  }
  async history(options: HistoryOptions = {}): Promise<models.HistoryResponse> {
    const {transport, id} = await this.context(options);
    return api.getVmHistory(transport, {...options, id}, options);
  }
  async list(path = '/', options: CallOptions & {depth?: number} = {}): Promise<models.FileListResponse> {
    const {transport, id} = await this.context(options);
    return api.listVmFiles(transport, {...options, id, ...(path === '/' ? {} : {path})}, options);
  }
  async changes(checkpoint: string, options: PageOptions = {}): Promise<models.ChangesResponse> {
    const {transport, id} = await this.context(options);
    return api.getVmChanges(transport, {...options, id, checkpoint}, options);
  }
  async timeline(options: TimelineOptions = {}): Promise<models.TimelineResponse> {
    const {transport, id} = await this.context(options);
    return api.getVmTimeline(transport, {...options, id}, options);
  }
}
