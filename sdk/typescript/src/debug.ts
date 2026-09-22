import * as api from './operations/index.js';
import * as models from './models/index.js';
import type {DiagnosticOptions, TriageOptions} from './options.js';
import type {Transport} from './transport.js';

export class Debug {
  constructor(private readonly transport: Transport) {}
  async panics(options: DiagnosticOptions = {}): Promise<models.PanicsResponse> {
    return api.getPanics(this.transport, options, options);
  }
  async triage(options: TriageOptions = {}): Promise<models.TriageResponse> {
    return api.getTriage(this.transport, {
      since: options.since ?? null, limit: options.limit ?? null, id: options.vm_id ?? null,
    }, options);
  }
}
