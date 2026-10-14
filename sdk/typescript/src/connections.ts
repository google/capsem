import * as api from './operations/index.js';
import type * as models from './models/index.js';
import type {VmContext} from './resources.js';
import type {CallOptions, Transport} from './transport.js';

/** Host consent never implicitly grants a VM access or launches a browser. */
export class Connections {
  constructor(private readonly transport: Transport) {}

  async list(options: CallOptions = {}): Promise<models.ConnectionListResponse> {
    return api.listConnections(this.transport, options);
  }

  /** Retain the request ID; opening the returned URL and polling are explicit. */
  async authorizeGoogle(request: models.AuthorizeGoogleRequest, options: CallOptions = {}): Promise<models.AuthorizeGoogleResponse> {
    return api.authorizeGoogleConnection(this.transport, {body: request}, options);
  }

  async authorization(id: string, options: CallOptions = {}): Promise<models.AuthorizationTransaction> {
    const result = await api.getConnectionAuthorization(this.transport, {transaction_id: id}, options);
    if (result.id !== id) throw new Error('authorization identity changed');
    return result;
  }

  async cancelAuthorization(id: string, options: CallOptions = {}): Promise<models.AuthorizationTransaction> {
    const result = await api.cancelConnectionAuthorization(this.transport, {transaction_id: id}, options);
    if (result.id !== id) throw new Error('authorization identity changed');
    return result;
  }

  async get(id: string, options: CallOptions = {}): Promise<models.ConnectionSummary> {
    const result = await api.getConnection(this.transport, {connection_id: id}, options);
    if (result.id !== id) throw new Error('connection identity changed');
    return result;
  }

  /** Fence grants; poll explicitly to confirm provider retirement. */
  async disconnect(id: string, options: CallOptions = {}): Promise<models.DisconnectResponse> {
    const result = await api.disconnectConnection(this.transport, {connection_id: id}, options);
    if (result.connection.id !== id) throw new Error('connection identity changed');
    return result;
  }
}

/** Explicit approvals for the canonical session resolved by this VM handle. */
export class CredentialGrants {
  constructor(private readonly context: (options: CallOptions) => Promise<VmContext>) {}

  async list(options: CallOptions = {}): Promise<models.SessionGrantListResponse> {
    const {transport, id} = await this.context(options);
    const result = await api.listSessionCredentialGrants(transport, {id}, options);
    if (result.session_id !== id || result.grants.some(grant =>
      grant.session_id !== id || grant.owner_generation !== result.owner_generation)) {
      throw new Error('grant session or ownership changed');
    }
    return result;
  }

  /** Never reconnect, acquire consent or replay a mutation implicitly. */
  async create(request: models.CreateSessionGrantRequest, options: CallOptions = {}): Promise<models.SessionGrantSummary> {
    const body = {...request, capability_ids: [...request.capability_ids]};
    const {transport, id} = await this.context(options);
    const result = await api.createSessionCredentialGrant(transport, {id, body}, options);
    const requested = new Set(body.capability_ids), returned = new Set(result.capability_ids);
    if (result.session_id !== id || result.owner_generation !== body.expected_owner_generation
      || result.connection_id !== body.connection_id || returned.size !== requested.size
      || returned.size !== result.capability_ids.length || [...requested].some(capability => !returned.has(capability))) {
      throw new Error('grant session, ownership, connection or capabilities changed');
    }
    return result;
  }

  async revoke(grant_id: string, options: CallOptions = {}): Promise<models.RevokeSessionGrantResponse> {
    const {transport, id} = await this.context(options);
    const result = await api.revokeSessionCredentialGrant(transport, {id, grant_id}, options);
    if (result.grant.session_id !== id || result.grant.id !== grant_id) throw new Error('revoked grant identity changed');
    return result;
  }
}
