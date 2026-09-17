import * as api from './operations/index.js';
import * as models from './models/index.js';
import type {NetworkLogOptions, PageOptions} from './options.js';
import type {CallOptions, Transport} from './transport.js';

export interface VmContext {transport: Transport; id: string}
class Resource {
  constructor(protected readonly context: (options: CallOptions) => Promise<VmContext>) {}
}

export class Files extends Resource {
  async read(path: string, options: CallOptions = {}): Promise<Uint8Array> {
    const {transport, id} = await this.context(options);
    return api.downloadVmFile(transport, {id, path}, options);
  }
  async write(path: string, data: Uint8Array, options: CallOptions = {}): Promise<models.UploadResponse> {
    const {transport, id} = await this.context(options);
    return api.uploadVmFile(transport, {id, path, body: data}, options);
  }
  async list(path = '/', options: CallOptions & {depth?: number} = {}): Promise<models.FileListResponse> {
    const {transport, id} = await this.context(options);
    return api.listVmFiles(transport, {...options, id, ...(path === '/' ? {} : {path})}, options);
  }
  async history(checkpoint: string, options: PageOptions = {}): Promise<models.ChangesResponse> {
    const {transport, id} = await this.context(options);
    return api.getVmChanges(transport, {...options, id, checkpoint}, options);
  }
}
export class Snapshots extends Resource {
  async list(options: CallOptions = {}): Promise<models.SnapshotsList> {
    const {transport, id} = await this.context(options);
    return api.listVmSnapshots(transport, {id}, options);
  }
  async status(options: CallOptions = {}): Promise<models.SnapshotsStatus> {
    const {transport, id} = await this.context(options);
    return api.getVmSnapshotsStatus(transport, {id}, options);
  }
}
export class Stats extends Resource {
  async summary(options: CallOptions = {}): Promise<models.VmStatsSummaryResponse> {
    const {transport, id} = await this.context(options);
    return api.getVmStatsSummary(transport, {id}, options);
  }
  async details(options: CallOptions = {}): Promise<models.VmStatsDetailResponse> {
    const {transport, id} = await this.context(options);
    return api.getVmStatsDetail(transport, {id}, options);
  }
}
export class Container extends Resource {
  async status(options: CallOptions = {}): Promise<models.ContainerStatusResponse> {
    const {transport, id} = await this.context(options);
    return api.getVmContainer(transport, {id}, options);
  }
}

export interface Port {
  id: string;
  guest: number;
  host: number | null;
  authenticate: boolean;
  url?: string;
  bootstrapToken?: string;
  expiresInSeconds?: number;
}

function port(exposure: models.ExposureInfo): Port {
  return {
    id: exposure.id, guest: exposure.guest_port, host: exposure.host_port ?? null,
    authenticate: exposure.access === models.ExposureAccess.HTTP_PREVIEW,
  };
}

export class Ports extends Resource {
  constructor(context: (options: CallOptions) => Promise<VmContext>,
              private readonly target: (options: CallOptions) => Promise<models.ExposureTarget>) {
    super(context);
  }
  async open(guest: number, options: CallOptions & {host?: number; authenticate?: boolean} = {}): Promise<Port> {
    if (!Number.isInteger(guest) || guest < 1 || guest > 65_535) throw new TypeError('Guest port must be between 1 and 65535');
    const host = options.host ?? 0, authenticate = options.authenticate ?? false;
    if (!Number.isInteger(host) || host < 0 || host > 65_535) throw new TypeError('Host port must be between 0 and 65535');
    if (authenticate && host !== 0) throw new TypeError('Authenticated ports cannot select a host port');
    const {transport, id} = await this.context(options);
    const exposure = await api.createVmExposure(transport, {id, body: {
      guest_port: guest, host_port: host, target: await this.target(options),
      access: authenticate ? models.ExposureAccess.HTTP_PREVIEW : models.ExposureAccess.LOOPBACK_TCP,
    }}, options);
    const opened = port(exposure);
    if (!authenticate) return opened;
    let session: models.PreviewSessionResponse;
    try {
      session = await api.createVmPreviewSession(transport, {id, exposure_id: exposure.id}, options);
    } catch (error) {
      // The caller never receives a Port to close. Remove the exposure without
      // the caller's signal (it may be the reason we failed) and report the
      // session failure.
      await api.deleteVmExposure(transport, {id, exposure_id: exposure.id}).catch(() => undefined);
      throw error;
    }
    return {...opened, host: null, url: session.url, bootstrapToken: session.bootstrap_token,
      expiresInSeconds: session.expires_in_seconds};
  }
  async list(options: CallOptions = {}): Promise<Port[]> {
    const {transport, id} = await this.context(options);
    const response = await api.listVmExposures(transport, {id}, options);
    return response.exposures.map(port);
  }
  async close(opened: Port, options: CallOptions = {}): Promise<models.VmActionResponse> {
    if (typeof opened !== 'object' || opened === null || typeof opened.id !== 'string') {
      throw new TypeError('Port must be an object returned by vm.ports');
    }
    const {transport, id} = await this.context(options);
    return api.deleteVmExposure(transport, {id, exposure_id: opened.id}, options);
  }
}

export class Networks {
  constructor(private readonly transport: Transport) {}
  async create(name: string, options: CallOptions = {}): Promise<models.NetworkInfo> {
    return api.createNetwork(this.transport, {body: {name}}, options);
  }
  async list(options: CallOptions = {}): Promise<models.NetworkInfo[]> {
    return (await api.listNetworks(this.transport, options)).networks;
  }
  async inspect(networkId: string, options: CallOptions = {}): Promise<models.NetworkInfo> {
    return api.getNetwork(this.transport, {id: networkId}, options);
  }
  async delete(network: models.NetworkInfo, options: CallOptions = {}): Promise<models.VmActionResponse> {
    return api.deleteNetwork(this.transport, {id: network.id}, options);
  }
  async logs(network: models.NetworkInfo, options: NetworkLogOptions = {}): Promise<models.NetworkLogsResponse> {
    return api.getNetworkLogs(this.transport, {...options, id: network.id}, options);
  }
}

export class VmNetworks extends Resource {
  async list(options: CallOptions = {}): Promise<models.NetworkInfo[]> {
    const {transport, id} = await this.context(options);
    const response = await api.listNetworks(transport, options);
    return response.networks.filter(network => network.members.some(member => member.vm_id === id));
  }
  async attach(network: models.NetworkInfo, options: CallOptions = {}): Promise<models.NetworkInfo> {
    const {transport, id} = await this.context(options);
    return api.attachNetworkMember(transport, {id: network.id, vm_id: id}, options);
  }
  async detach(network: models.NetworkInfo, options: CallOptions = {}): Promise<models.NetworkInfo> {
    const {transport, id} = await this.context(options);
    return api.detachNetworkMember(transport, {id: network.id, vm_id: id}, options);
  }
}

export class McpTools {
  constructor(private readonly transport: Transport, private readonly profileId: string,
              private readonly serverId: string) {}
  async list(options: CallOptions = {}): Promise<models.McpToolsListResponse> {
    return api.listProfileMcpTools(this.transport, {profile_id: this.profileId, server_id: this.serverId}, options);
  }
  async call(name: string, arguments_: models.Value, options: CallOptions = {}): Promise<models.Value> {
    return api.callProfileMcpTool(this.transport, {
      profile_id: this.profileId, server_id: this.serverId, tool_id: name, body: arguments_,
    }, options);
  }
}

export class ProfileMcpServer {
  readonly tools: McpTools;
  constructor(private readonly transport: Transport, private readonly profileId: string,
              readonly info: models.McpServerInfoResponse) {
    this.tools = new McpTools(transport, profileId, info.name);
  }
  async refresh(options: CallOptions = {}): Promise<models.McpRefreshResponse> {
    return api.refreshProfileMcpServer(this.transport, {
      profile_id: this.profileId, server_id: this.info.name,
    }, options);
  }
}

export class ProfileMcp {
  constructor(private readonly transport: Transport, private readonly profile: models.ProfileSummary) {}
  async info(options: CallOptions = {}): Promise<models.ProfileMcpInfoResponse> {
    return api.getProfileMcpInfo(this.transport, {profile_id: this.profile.id}, options);
  }
  async servers(options: CallOptions = {}): Promise<models.McpServersListResponse> {
    return api.listProfileMcpServers(this.transport, {profile_id: this.profile.id}, options);
  }
  async defaultPermission(options: CallOptions = {}): Promise<models.McpDefaultPermissionResponse> {
    return api.getProfileMcpDefault(this.transport, {profile_id: this.profile.id}, options);
  }
  async get(name: string, options: CallOptions = {}): Promise<ProfileMcpServer> {
    const matches = (await this.servers(options)).filter(server => server.name === name);
    if (matches.length !== 1) throw new TypeError(`Expected one MCP server named ${JSON.stringify(name)}, found ${matches.length}`);
    return new ProfileMcpServer(this.transport, this.profile.id, matches[0] as models.McpServerInfoResponse);
  }
}

export class Profiles {
  constructor(private readonly transport: Transport) {}
  async list(options: CallOptions = {}): Promise<models.ProfileSummary[]> {
    return (await api.listProfiles(this.transport, options)).profiles;
  }
  mcp(profile: models.ProfileSummary): ProfileMcp {
    if (typeof profile !== 'object' || profile === null || typeof profile.id !== 'string') {
      throw new TypeError('Profile must be an object returned by hypervisor.profiles');
    }
    return new ProfileMcp(this.transport, profile);
  }
}
