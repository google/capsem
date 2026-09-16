import * as api from './operations/index.js';
import * as models from './models/index.js';
import type {NetworkLogOptions} from './options.js';
import type {CallOptions, Transport} from './transport.js';

export interface VmContext {transport: Transport; id: string}
class Resource {
  constructor(protected readonly context: (options: CallOptions) => Promise<VmContext>) {}
}

export class Copy extends Resource {
  async fromVm(path: string, options: CallOptions = {}): Promise<Uint8Array> {
    const {transport, id} = await this.context(options);
    return api.downloadVmFile(transport, {id, path}, options);
  }
  async toVm(path: string, data: Uint8Array, options: CallOptions = {}): Promise<models.UploadResponse> {
    const {transport, id} = await this.context(options);
    return api.uploadVmFile(transport, {id, path, body: data}, options);
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
    const session = await api.createVmPreviewSession(transport, {id, exposure_id: exposure.id}, options);
    return {...opened, host: null, url: session.url, bootstrapToken: session.bootstrap_token,
      expiresInSeconds: session.expires_in_seconds};
  }
  async list(options: CallOptions = {}): Promise<Port[]> {
    const {transport, id} = await this.context(options);
    const response = await api.listVmExposures(transport, {id}, options);
    return response.exposures.map(port);
  }
  async close(opened: Port | string, options: CallOptions = {}): Promise<models.VmActionResponse> {
    const {transport, id} = await this.context(options);
    const exposureId = typeof opened === 'string' ? opened : opened.id;
    return api.deleteVmExposure(transport, {id, exposure_id: exposureId}, options);
  }
}

export class Networks {
  constructor(private readonly transport: Transport) {}
  async create(name: string, options: CallOptions = {}): Promise<models.NetworkInfo> {
    return api.createNetwork(this.transport, {body: {name}}, options);
  }
  async list(options: CallOptions = {}): Promise<models.NetworkListResponse> {
    return api.listNetworks(this.transport, options);
  }
  async inspect(networkId: string, options: CallOptions = {}): Promise<models.NetworkInfo> {
    return api.getNetwork(this.transport, {id: networkId}, options);
  }
  async delete(networkId: string, options: CallOptions = {}): Promise<models.VmActionResponse> {
    return api.deleteNetwork(this.transport, {id: networkId}, options);
  }
  async attach(networkId: string, vmId: string, options: CallOptions = {}): Promise<models.NetworkInfo> {
    return api.attachNetworkMember(this.transport, {id: networkId, vm_id: vmId}, options);
  }
  async detach(networkId: string, vmId: string, options: CallOptions = {}): Promise<models.NetworkInfo> {
    return api.detachNetworkMember(this.transport, {id: networkId, vm_id: vmId}, options);
  }
  async logs(networkId: string, options: NetworkLogOptions = {}): Promise<models.NetworkLogsResponse> {
    return api.getNetworkLogs(this.transport, {...options, id: networkId}, options);
  }
}

export class ProfileMcp {
  constructor(private readonly transport: Transport, private readonly profileId: string) {}
  async info(options: CallOptions = {}): Promise<models.ProfileMcpInfoResponse> {
    return api.getProfileMcpInfo(this.transport, {profile_id: this.profileId}, options);
  }
  async servers(options: CallOptions = {}): Promise<models.McpServersListResponse> {
    return api.listProfileMcpServers(this.transport, {profile_id: this.profileId}, options);
  }
  async defaultPermission(options: CallOptions = {}): Promise<models.McpDefaultPermissionResponse> {
    return api.getProfileMcpDefault(this.transport, {profile_id: this.profileId}, options);
  }
  async tools(serverId: string, options: CallOptions = {}): Promise<models.McpToolsListResponse> {
    return api.listProfileMcpTools(this.transport, {profile_id: this.profileId, server_id: serverId}, options);
  }
  async refresh(serverId: string, options: CallOptions = {}): Promise<models.McpRefreshResponse> {
    return api.refreshProfileMcpServer(this.transport, {profile_id: this.profileId, server_id: serverId}, options);
  }
  async call(serverId: string, toolId: string, arguments_: models.Value, options: CallOptions = {}): Promise<models.Value> {
    return api.callProfileMcpTool(this.transport, {
      profile_id: this.profileId, server_id: serverId, tool_id: toolId, body: arguments_,
    }, options);
  }
}

export class Profiles {
  constructor(private readonly transport: Transport) {}
  async list(options: CallOptions = {}): Promise<models.ProfilesListResponse> {
    return api.listProfiles(this.transport, options);
  }
  mcp(profileId: string): ProfileMcp {return new ProfileMcp(this.transport, profileId);}
}
