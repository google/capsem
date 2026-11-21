import * as api from './operations/index.js';
import type * as models from './models/index.js';
import type {CallOptions, Transport} from './transport.js';

export interface RequestListOptions extends CallOptions {
  after?: number;
  limit?: number;
  pendingOnly?: boolean;
}

/** Operator inspection of durable requests and their frozen review inputs. */
export class Requests {
  constructor(private readonly transport: Transport) {}

  async list(options: RequestListOptions = {}): Promise<models.RequestListResponse> {
    return api.listRequests(this.transport, {
      ...(options.after === undefined ? {} : {after: options.after}),
      ...(options.limit === undefined ? {} : {limit: options.limit}),
      ...(options.pendingOnly === undefined ? {} : {pending_only: options.pendingOnly}),
    }, options);
  }

  async info(id: string, options: CallOptions = {}): Promise<models.RequestDetailResponse> {
    const result = await api.getRequestInfo(this.transport, {id}, options);
    if (result.summary.request_id !== id) throw new Error('Review request identity changed');
    return result;
  }
}
