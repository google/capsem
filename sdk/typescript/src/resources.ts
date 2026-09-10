import * as api from './operations/index.js';
import type * as models from './models/index.js';
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
