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

export class Exposures extends Resource {
  async create(request: models.ExposureRequest, options: CallOptions = {}): Promise<models.ExposureInfo> {
    const {transport, id} = await this.context(options);
    return api.createVmExposure(transport, {id, body: request}, options);
  }
  async list(options: CallOptions = {}): Promise<models.ExposureListResponse> {
    const {transport, id} = await this.context(options);
    return api.listVmExposures(transport, {id}, options);
  }
  async delete(exposureId: string, options: CallOptions = {}): Promise<models.VmActionResponse> {
    const {transport, id} = await this.context(options);
    return api.deleteVmExposure(transport, {id, exposure_id: exposureId}, options);
  }
  async previewSession(exposureId: string, options: CallOptions = {}): Promise<models.PreviewSessionResponse> {
    const {transport, id} = await this.context(options);
    return api.createVmPreviewSession(transport, {id, exposure_id: exposureId}, options);
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
