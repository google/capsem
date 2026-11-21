import * as api from './operations/index.js';
import type * as models from './models/index.js';
import type {CallOptions, Transport} from './transport.js';

export interface EnvironmentListOptions extends CallOptions {
  search?: string;
  after?: string;
  limit?: number;
}

/** Canonical blueprints; requests retain caller-chosen reconciliation IDs. */
export class Environments {
  constructor(private readonly transport: Transport) {}

  /** List visible environments. Follow next_cursor even after an empty search page. */
  async list({signal, timeoutMs, ...query}: EnvironmentListOptions = {}): Promise<models.EnvironmentListResponse> {
    return api.listEnvironments(this.transport, query, {signal, timeoutMs});
  }

  /** Approval can precede application. Reconcile explicitly after interruption. */
  async create(request: models.EnvironmentCreateRequest, options: CallOptions = {}): Promise<models.EnvironmentMutationResponse> {
    const requestId = request.request_id;
    const result = await api.createEnvironment(this.transport, {body: request}, options);
    if (result.request_id !== requestId) throw new Error('Environment request identity changed');
    return result;
  }

  async change(id: string, request: models.EnvironmentChangeRequest, options: CallOptions = {}): Promise<models.EnvironmentMutationResponse> {
    const requestId = request.request_id;
    const result = await api.changeEnvironment(this.transport, {id, body: request}, options);
    if (result.request_id !== requestId || result.environment_id !== id) throw new Error('Environment change identity changed');
    return result;
  }

  async info(id: string, options: CallOptions = {}): Promise<models.EnvironmentCurrentResponse> {
    const result = await api.getEnvironmentInfo(this.transport, {id}, options);
    if (result.document.identity.id !== id) throw new Error('Environment identity changed');
    return result;
  }
}
