/** Immutable timing data is read before, and grants no authority for, creation. */
import * as api from './operations/index.js';
import type {ProvisionRequest} from './models/index.js';
import {createDeadlineMs} from './execution.js';
import type {CallOptions, Transport} from './transport.js';

export async function creationDeadlineMs(transport: Transport, request: ProvisionRequest, options: CallOptions = {}): Promise<number> {
  let seconds = 0;
  if (request.environment != null && request.attach !== true) {
    const selected = request.environment;
    const response = await api.getEnvironmentReadiness(transport, {id: selected.environment_id, revision_id: selected.revision_id}, options);
    if (response.environment.environment_id !== selected.environment_id || response.environment.revision_id !== selected.revision_id) {
      throw new Error('Environment readiness identity changed');
    }
    seconds = response.startup_timeout_seconds;
  }
  const planned = createDeadlineMs(transport.timeoutMs, seconds);
  return options.timeoutMs ?? planned;
}
