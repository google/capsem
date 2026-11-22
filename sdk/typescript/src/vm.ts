import {Client} from './client.js';
import {CredentialGrants} from './connections.js';
import {commandDeadlineMs} from './execution.js';
import type {EnvironmentListOptions} from './environments.js';
import * as api from './operations/index.js';
import * as models from './models/index.js';
import type {HistoryOptions, LogOptions, TimelineOptions, VmSelector} from './options.js';
import {lifecycleReadiness, prepareStart} from './readiness.js';
import {Container, Files, Ports, Stats, VmNetworks, type VmContext} from './resources.js';
import {HttpError, Transport, type CallOptions, type TransportOptions} from './transport.js';
import {ExecRequestSchema} from './validation/ExecRequest.js';
import {ForkRequestSchema} from './validation/ForkRequest.js';

export class VM extends Client {
  #id: string | undefined;
  #name: string | undefined;
  #hasContainer: boolean | undefined;
  readonly files: Files;
  readonly networks: VmNetworks;
  readonly stats: Stats;
  readonly container: Container;
  readonly ports: Ports;
  readonly credentialGrants: CredentialGrants;

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
    this.files = new Files(context);
    this.networks = new VmNetworks(context);
    this.stats = new Stats(context);
    this.container = new Container(context);
    this.ports = new Ports(context, call => this.portTarget(call));
    this.credentialGrants = new CredentialGrants(context);
  }
  /** @internal */
  static bind(transport: Transport, id: string, name: string, hasContainer?: boolean): VM {
    const vm = new VM(transport, {id});
    vm.#name = name;
    vm.#hasContainer = hasContainer;
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
  private async portTarget(options: CallOptions): Promise<models.ExposureTarget> {
    if (this.#hasContainer === undefined) {
      const {transport, id} = await this.context(options);
      try {
        await api.getVmContainer(transport, {id}, options);
        this.#hasContainer = true;
      } catch (error) {
        if (!(error instanceof HttpError) || error.status !== 404) throw error;
        this.#hasContainer = false;
      }
    }
    return this.#hasContainer ? models.ExposureTarget.CONTAINER : models.ExposureTarget.VM;
  }
  async info(options: CallOptions = {}): Promise<models.SandboxInfo> {
    const {transport, id} = await this.context(options);
    return api.getVmInfo(transport, {id}, options);
  }
  /** List the MCP servers reported by the running VM. */
  async mcps(options: CallOptions = {}): Promise<models.VmMcpServersResponse> {
    const {transport, id} = await this.context(options);
    const result = await api.listVmMcps(transport, {id}, options);
    if (result.vm_id !== id) throw new Error('Sandbox MCP inventory identity changed');
    return result;
  }
  /** List definitions reported by one of the running VM's MCP servers. */
  async mcpTools(serverId: string, options: CallOptions = {}): Promise<models.VmMcpToolsResponse> {
    const {transport, id} = await this.context(options);
    const result = await api.listVmMcpTools(transport, {id, server_id: serverId}, options);
    if (result.vm_id !== id || result.server_id !== serverId) {
      throw new Error('Sandbox MCP inventory identity changed');
    }
    return result;
  }
  /** List selections applied to the running VM's boot revision. */
  async skills({revisionId, signal, timeoutMs, ...query}: EnvironmentListOptions & {revisionId?: string} = {}): Promise<models.VmSkillListResponse> {
    const options = {signal, timeoutMs};
    const {transport, id} = await this.context(options);
    const result = await api.listVmSkills(transport, {...query, id,
      ...(revisionId === undefined ? {} : {revision_id: revisionId}),
    }, options);
    if (result.vm_id !== id || (revisionId !== undefined && result.revision?.revision_id !== revisionId)) {
      throw new Error('Sandbox skill revision identity changed');
    }
    return result;
  }
  /** Read the complete pinned skill dictionary for the running VM. */
  async skill(skillId: string, {revisionId, ...options}: CallOptions & {revisionId?: string} = {}): Promise<models.VmSkillDetailResponse> {
    const {transport, id} = await this.context(options);
    const result = await api.getVmSkill(transport, {id, skill_id: skillId,
      ...(revisionId === undefined ? {} : {revision_id: revisionId}),
    }, options);
    const {skill} = result;
    if (result.vm_id !== id || (revisionId !== undefined && skill.revision.revision_id !== revisionId)
      || skill.selection.id !== skillId || skill.metadata.id !== skillId || skill.selection.version !== skill.metadata.version) {
      throw new Error('Sandbox skill identity changed');
    }
    return result;
  }
  async exec(command: string, options: CallOptions & {timeout_secs?: number; target?: models.ExecTarget | null} = {}): Promise<models.ExecResponse> {
    const body = ExecRequestSchema.parse({command, timeout_secs: options.timeout_secs ?? null,
      ...(options.target == null ? {} : {target: options.target})});
    const {transport, id} = await this.context(options);
    return api.execVm(transport, {id, body}, {
      ...options, timeoutMs: options.timeoutMs ?? commandDeadlineMs(transport.timeoutMs, options.timeout_secs),
    });
  }
  async start(options: CallOptions = {}): Promise<models.ProvisionResponse> {
    const {transport, id} = await this.context(options);
    const {body, timeoutMs} = await prepareStart(transport, id, options);
    return api.startVm(transport, {id, body}, {...options, timeoutMs});
  }
  async persist(name: string, options: CallOptions = {}): Promise<models.PersistResponse> {
    const {transport, id} = await this.context(options);
    return api.persistVm(transport, {id, body: {name}}, options);
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
    const {body, timeoutMs} = await prepareStart(transport, id, options);
    return api.resumeVm(transport, {id, body}, {...options, timeoutMs});
  }
  async delete(options: CallOptions = {}): Promise<models.VmActionResponse> {
    const {transport, id} = await this.context(options);
    return api.deleteVm(transport, {id}, options);
  }
  async fork(name: string, options: CallOptions & {description?: string; labels?: Record<string, string>} = {}): Promise<VM> {
    const body = ForkRequestSchema.parse({
      name, description: options.description ?? null,
      ...(options.labels === undefined ? {} : {labels: options.labels}),
    });
    const {transport, id} = await this.context(options);
    const {selection} = await lifecycleReadiness(transport, id, models.VmLifecycleIntent.FORK, options);
    const response = await api.forkVm(transport, {id, body: {...body, expected: selection}}, options);
    return VM.bind(transport, response.id, response.name, selection.environment != null ? undefined : this.#hasContainer);
  }
  async log(options: LogOptions = {}): Promise<models.LogsResponse> {
    const {transport, id} = await this.context(options);
    return api.getVmLogs(transport, {...options, id}, options);
  }
  async history(options: HistoryOptions = {}): Promise<models.HistoryResponse> {
    const {transport, id} = await this.context(options);
    return api.getVmHistory(transport, {...options, id}, options);
  }
  async timeline(options: TimelineOptions = {}): Promise<models.TimelineResponse> {
    const {transport, id} = await this.context(options);
    return api.getVmTimeline(transport, {...options, id}, options);
  }
}
