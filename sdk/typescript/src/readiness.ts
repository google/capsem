/** Exact VM selections and approved startup timing are read before a mutation. */
import * as api from './operations/index.js';
import {VmLifecycleIntent, type ProvisionRequest, type VmLifecycleSelection, type VmReadinessResponse, type VmStartRequest} from './models/index.js';
import {createDeadlineMs} from './execution.js';
import {HttpError, type CallOptions, type Transport} from './transport.js';

const VM_NAME = /^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/;
const CANONICAL_UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const NIL_UUID = '00000000-0000-0000-0000-000000000000';
const CHANGED_SELECTION = '{"error":"lifecycle selection changed; request a new readiness view"}';

function validUuid(value: string): boolean {
  return value !== NIL_UUID && CANONICAL_UUID.test(value);
}

function sameSelection(a: VmLifecycleSelection, b: VmLifecycleSelection): boolean {
  return a.id === b.id && a.running === b.running && (a.generation ?? null) === (b.generation ?? null)
    && a.environment?.environment_id === b.environment?.environment_id
    && a.environment?.revision_id === b.environment?.revision_id;
}

export async function lifecycleReadiness(transport: Transport, id: string, intent: VmLifecycleIntent,
    options: CallOptions, expected?: VmLifecycleSelection | null): Promise<VmReadinessResponse> {
  if (typeof id !== 'string' || !VM_NAME.test(id)) throw new TypeError('Invalid VM identifier');
  const readiness = await api.getVmReadiness(transport, {id, intent}, options).catch((error: unknown) => {
    if (expected != null && error instanceof HttpError && error.status === 404) throw new HttpError(409, CHANGED_SELECTION);
    throw error;
  });
  const {selection, startup_timeout_seconds} = readiness;
  if (intent !== VmLifecycleIntent.CLONE && selection.id !== id) throw new Error('Lifecycle readiness targets a different VM');
  if (!VM_NAME.test(selection.id)) throw new TypeError('Invalid VM identifier in lifecycle readiness');
  if (selection.generation == null ? selection.running : !validUuid(selection.generation)) {
    throw new TypeError('Invalid lifecycle generation');
  }
  if (selection.environment != null
    && (!validUuid(selection.environment.environment_id) || !validUuid(selection.environment.revision_id))) {
    throw new TypeError('Invalid lifecycle environment reference');
  }
  if (!Number.isInteger(startup_timeout_seconds) || startup_timeout_seconds < 0 || startup_timeout_seconds > 2 ** 32 - 1) {
    throw new TypeError('Invalid approved startup timeout');
  }
  if (expected != null && !sameSelection(expected, selection)) throw new HttpError(409, CHANGED_SELECTION);
  return readiness;
}

export async function prepareStart(transport: Transport, id: string, options: CallOptions): Promise<{body: VmStartRequest; timeoutMs: number}> {
  const readiness = await lifecycleReadiness(transport, id, VmLifecycleIntent.START, options);
  const seconds = readiness.selection.environment == null ? 0 : readiness.startup_timeout_seconds;
  return {body: {expected: readiness.selection}, timeoutMs: options.timeoutMs ?? createDeadlineMs(transport.timeoutMs, seconds)};
}

export async function creationDeadlineMs(transport: Transport, request: ProvisionRequest, options: CallOptions = {}): Promise<number> {
  let seconds = 0;
  if (request.from != null) {
    const readiness = await lifecycleReadiness(transport, request.from, VmLifecycleIntent.CLONE, options, request.expected_source);
    request.expected_source = readiness.selection;
    if (request.container == null && readiness.selection.environment != null) seconds = readiness.startup_timeout_seconds;
  }
  if (request.environment != null && request.attach !== true) {
    const selected = request.environment;
    const response = await api.getEnvironmentReadiness(transport, {id: selected.environment_id, revision_id: selected.revision_id}, options);
    if (response.environment.environment_id !== selected.environment_id || response.environment.revision_id !== selected.revision_id) {
      throw new Error('Environment readiness identity changed');
    }
    seconds = response.startup_timeout_seconds;
  }
  const planned = createDeadlineMs(transport.timeoutMs, request.attach === true ? 0 : seconds);
  return options.timeoutMs ?? planned;
}
