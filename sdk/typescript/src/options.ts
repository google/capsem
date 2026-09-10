import type {HistoryLayerFilter, HostLogSource, TimelineLayer} from './models/index.js';
import type {CallOptions} from './transport.js';

export type VmSelector = {id: string; name?: never} | {name: string; id?: never};
export interface CreateOptions extends CallOptions {
  name?: string; vcpu?: number; memory?: string | number; env?: Record<string, string>;
}
export interface LogOptions extends CallOptions {grep?: string; tail?: number; max_bytes?: number}
export interface HostLogOptions extends LogOptions {source?: HostLogSource}
export interface HistoryOptions extends CallOptions {
  limit?: number; offset?: number; search?: string; layer?: HistoryLayerFilter;
}
export interface TimelineOptions extends CallOptions {
  trace_id?: string; since?: string; limit?: number; layers?: TimelineLayer[];
}
export interface PageOptions extends CallOptions {limit?: number; offset?: number}
