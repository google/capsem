import {ConnectionState, GrantState, TransactionState} from '@capsem/sdk';
import type {AuthorizeGoogleRequest, AuthorizationTransaction, ConnectionListResponse, ConnectionSummary,
  CreateSessionGrantRequest, SessionGrantListResponse} from '@capsem/sdk';
import type * as api from '../api';

export type CredentialFlowApi = Pick<typeof api, 'listConnections' | 'authorizeGoogleConnection' |
  'getConnectionAuthorization' | 'cancelConnectionAuthorization' | 'getConnection' | 'disconnectConnection' |
  'listSessionCredentialGrants' | 'createSessionCredentialGrant' | 'revokeSessionCredentialGrant'>;

type Feedback = {busy: boolean; error: string | null};
class Flow<State extends Feedback> {
  private closed = false;
  constructor(public state: State, private readonly changed: (state: State) => void) {}
  protected update(patch: Partial<State>) {
    if (this.closed) return;
    this.state = {...this.state, ...patch};
    this.changed(this.state);
  }
  protected async run(task: () => Promise<void>, failure: string) {
    if (this.closed || this.state.busy) return;
    this.update({busy: true, error: null} as Partial<State>);
    try { await task(); }
    catch { this.update({error: failure} as Partial<State>); }
    finally { this.update({busy: false} as Partial<State>); }
  }
  dispose() { this.closed = true; }
}

export interface ConnectionViewState extends Feedback {
  catalog: ConnectionListResponse | null;
  transaction: AuthorizationTransaction | null;
  url: string | null;
}
export class ConnectionFlow extends Flow<ConnectionViewState> {
  private request: {key: string; body: AuthorizeGoogleRequest} | null = null;
  constructor(private readonly api: CredentialFlowApi, changed: (state: ConnectionViewState) => void,
    private readonly requestId: () => string = () => crypto.randomUUID()) {
    super({catalog: null, transaction: null, url: null, busy: false, error: null}, changed);
  }
  load() {
    return this.run(async () => {
      this.update({catalog: null});
      this.update({catalog: await this.api.listConnections()});
    }, 'Could not load connections. Refresh to try again.');
  }
  authorize(profileId: string, reconnect: string | null = null) {
    return this.run(async () => {
      const profile = this.state.catalog?.scope_profiles.find(p => p.id === profileId);
      const original = this.state.catalog?.connections.find(c => c.id === reconnect);
      if (!profile?.enabled || !profile.capabilities.some(c => c.supported) ||
        (reconnect && original?.scope_profile_id !== profileId) || this.state.transaction?.state === TransactionState.PENDING) {
        throw new Error('Unavailable consent');
      }
      const key = JSON.stringify([profileId, reconnect]);
      if (this.request?.key !== key) this.request = {key, body: {
        request_id: this.requestId(), scope_profile_id: profileId, reconnect_connection_id: reconnect,
      }};
      const response = await this.api.authorizeGoogleConnection(this.request.body);
      this.update({transaction: response.transaction, url: null});
      if (response.transaction.state !== TransactionState.PENDING) {
        this.request = null;
        this.update({catalog: await this.api.listConnections()});
        return;
      }
      if (response.authorization_url) {
        const url = new URL(response.authorization_url);
        if (url.protocol !== 'https:' || url.hostname !== 'accounts.google.com' || url.port || url.username || url.password || url.hash) {
          throw new Error('Invalid consent URL');
        }
        this.update({url: url.href});
      }
    }, 'Sign-in was not confirmed. Retry the same action or cancel the pending sign-in.');
  }
  poll() {
    return this.run(async () => {
      if (!this.state.transaction) return;
      const transaction = await this.api.getConnectionAuthorization(this.state.transaction.id);
      this.update({transaction});
      if (transaction.state !== TransactionState.PENDING) {
        this.request = null;
        this.update({url: null, catalog: await this.api.listConnections()});
      }
    }, 'Could not check sign-in. Check again or cancel.');
  }
  cancel() {
    return this.run(async () => {
      if (!this.state.transaction) return;
      const transaction = await this.api.cancelConnectionAuthorization(this.state.transaction.id);
      this.update({transaction, url: null});
      this.request = null;
    }, 'Cancellation was not confirmed. Check sign-in or retry cancellation.');
  }
  private replace(connection: ConnectionSummary) {
    const catalog = this.state.catalog;
    if (catalog) this.update({catalog: {...catalog, connections: catalog.connections.map(c => c.id === connection.id ? connection : c)}});
  }
  disconnect(id: string) {
    return this.run(async () => {
      this.replace((await this.api.disconnectConnection(id)).connection);
    }, 'Disconnect was not confirmed. Check status before retrying.');
  }
  check(id: string) {
    return this.run(async () => { this.replace(await this.api.getConnection(id)); }, 'Could not check connection status.');
  }
}

export function availableCapabilities(catalog: ConnectionListResponse | null, connectionId: string, denied: string[]) {
  const connection = catalog?.connections.find(c => c.id === connectionId);
  if (!connection || connection.state !== ConnectionState.CONNECTED) return [];
  const profile = catalog?.scope_profiles.find(p => p.id === connection.scope_profile_id);
  if (!profile?.enabled) return [];
  return profile.capabilities.filter(c => c.supported && connection.available_capabilities.includes(c.id) && !denied.includes(c.id));
}

export function capabilityLabel(catalog: ConnectionListResponse | null, connectionId: string, capabilityId: string) {
  const connection = catalog?.connections.find(c => c.id === connectionId);
  const profile = catalog?.scope_profiles.find(p => p.id === connection?.scope_profile_id);
  return profile?.capabilities.find(c => c.id === capabilityId)?.label ?? capabilityId;
}

export interface SessionGrantViewState extends Feedback {
  catalog: ConnectionListResponse | null;
  grants: SessionGrantListResponse | null;
}
export class SessionGrantFlow extends Flow<SessionGrantViewState> {
  private request: {key: string; body: CreateSessionGrantRequest} | null = null;
  constructor(private readonly session: string, private readonly api: CredentialFlowApi,
    changed: (state: SessionGrantViewState) => void, private readonly requestId: () => string = () => crypto.randomUUID(),
    private readonly canGrant = true) {
    super({catalog: null, grants: null, busy: false, error: null}, changed);
  }
  load() {
    return this.run(async () => {
      this.update({catalog: null, grants: null});
      if (!this.canGrant) {
        this.update({grants: await this.api.listSessionCredentialGrants(this.session)});
        return;
      }
      const [catalog, grants] = await Promise.all([this.api.listConnections(), this.api.listSessionCredentialGrants(this.session)]);
      this.update({catalog, grants});
    }, 'Could not load session access. The session or policy may have changed.');
  }
  grant(connectionId: string, capabilities: string[]) {
    return this.run(async () => {
      const current = this.state.grants;
      if (!this.canGrant) throw new Error('Retained approval is read-only');
      const allowed = availableCapabilities(this.state.catalog, connectionId, current?.denied_capability_ids ?? []);
      const selected = [...new Set(capabilities)].sort();
      if (!current || !selected.length || selected.some(id => !allowed.some(c => c.id === id))) throw new Error('Unavailable capability');
      const key = JSON.stringify([current.owner_generation, connectionId, selected]);
      if (this.request?.key !== key) this.request = {key, body: {
        request_id: this.requestId(), expected_owner_generation: current.owner_generation,
        connection_id: connectionId, capability_ids: selected, expires_at: null,
      }};
      const grant = await this.api.createSessionCredentialGrant(this.session, this.request.body);
      this.request = null;
      this.update({grants: {...current, grants: [...current.grants.filter(g => g.id !== grant.id), grant]}});
    }, 'Access was not confirmed. Refresh session access or retry the same selection.');
  }
  revoke(id: string) {
    return this.run(async () => {
      if (!this.state.grants?.grants.some(g => g.id === id && g.state === GrantState.ACTIVE)) throw new Error('Unavailable grant');
      const {grant} = await this.api.revokeSessionCredentialGrant(this.session, id);
      const current = this.state.grants;
      if (current) this.update({grants: {...current, grants: current.grants.map(g => g.id === id ? grant : g)}});
    }, 'Revocation was not confirmed. Refresh access before retrying.');
  }
}
