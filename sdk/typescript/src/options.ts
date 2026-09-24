import type {HistoryLayerFilter, HostLogSource, NetworkInfo, ProfileSummary, TimelineLayer} from './models/index.js';
import type {CallOptions} from './transport.js';

/**
 * What a created sandbox runs. A container brings its own userland, so the
 * catalog answers its default profile apart from a VM's.
 */
export type Runtime = 'vm' | 'container';

export type VmSelector = {id: string; name?: never} | {name: string; id?: never};
export interface Registry {username?: string; password?: string; ca_pem?: string}
export interface CreateOptions extends CallOptions {
  profile?: ProfileSummary; name?: string; cpus?: number; memory?: number;
  env?: Record<string, string>; networks?: readonly NetworkInfo[];
  image?: string; command?: readonly string[]; registry?: Registry;
}
export interface RunOptions extends CallOptions {
  profile?: ProfileSummary; timeout_secs?: number; cpus?: number; memory?: number; env?: Record<string, string>;
}
export interface DiagnosticOptions extends CallOptions {since?: string; limit?: number}
export interface TriageOptions extends DiagnosticOptions {vm_id?: string}
export interface LogOptions extends CallOptions {grep?: string; tail?: number; max_bytes?: number}
export interface HostLogOptions extends LogOptions {source?: HostLogSource}
export interface HistoryOptions extends CallOptions {
  limit?: number; offset?: number; search?: string; layer?: HistoryLayerFilter;
}
export interface TimelineOptions extends CallOptions {
  trace_id?: string; since?: string; limit?: number; layers?: TimelineLayer[];
}
export interface NetworkLogOptions extends CallOptions {
  cursor?: string; limit?: number; vm?: string; connection?: string; type?: string;
  decision?: string; since?: number; until?: number;
}
