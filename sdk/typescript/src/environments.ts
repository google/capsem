import * as api from './operations/index.js';
import type * as models from './models/index.js';
import type {CallOptions, Transport} from './transport.js';

export interface EnvironmentListOptions extends CallOptions {
  search?: string;
  after?: string;
  limit?: number;
}

export interface EnvironmentRevisionListOptions extends CallOptions {
  before?: number;
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

  async listRevisions(id: string, {signal, timeoutMs, ...query}: EnvironmentRevisionListOptions = {}): Promise<models.EnvironmentRevisionListResponse> {
    return api.listEnvironmentRevisions(this.transport, {id, ...query}, {signal, timeoutMs});
  }

  async revision(reference: models.EnvironmentRevisionReference, options: CallOptions = {}): Promise<models.EnvironmentRevisionDetailResponse> {
    const result = await api.getEnvironmentRevision(this.transport, {id: reference.environment_id, revision_id: reference.revision_id}, options);
    verifyRevision(reference, result.summary.environment);
    return result;
  }

  /** Read plugins for an exact current revision, or defaults for creation. */
  async plugins(reference: models.EnvironmentRevisionReference | null = null, options: CallOptions = {}): Promise<models.EnvironmentPluginListResponse> {
    const result = reference
      ? await api.listEnvironmentPlugins(this.transport, {id: reference.environment_id, revision_id: reference.revision_id}, options)
      : await api.listNewEnvironmentPlugins(this.transport, options);
    if (reference === null ? result.revision !== null : result.revision === null) {
      throw new Error('Environment plugin revision identity changed');
    }
    if (reference && result.revision) verifyRevision(reference, result.revision);
    return result;
  }

  async validateToml(base: models.EnvironmentRevisionReference, toml: string, options: CallOptions = {}): Promise<models.EnvironmentTomlValidationResponse> {
    const result = await api.validateEnvironmentToml(this.transport, {id: base.environment_id, revision_id: base.revision_id, body: {toml}}, options);
    verifyRevision(base, result.base);
    return result;
  }

  /** Read the rules compiled for an exact current environment revision. */
  async rules(reference: models.EnvironmentRevisionReference, options: CallOptions = {}): Promise<models.EnvironmentRuleListResponse> {
    const result = await api.listEnvironmentRules(this.transport, {id: reference.environment_id, revision_id: reference.revision_id}, options);
    verifyRevision(reference, result.revision);
    return result;
  }
}

function verifyRevision(expected: models.EnvironmentRevisionReference, actual: models.EnvironmentRevisionReference): void {
  if (expected.environment_id !== actual.environment_id || expected.revision_id !== actual.revision_id) {
    throw new Error('Environment revision identity changed');
  }
}
