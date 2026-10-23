import type { VmSummary } from '@capsem/sdk';

export type SessionSortKey = 'name' | 'status' | 'uptime' | 'tokens' | 'cost';
export type SessionSortDirection = 'asc' | 'desc';
export type SortableSession = Pick<VmSummary,
  'id' | 'name' | 'status' | 'uptime_secs' | 'total_input_tokens' | 'total_output_tokens' | 'total_estimated_cost'>;

export function sessionTokens(session: Pick<VmSummary, 'total_input_tokens' | 'total_output_tokens'>): number | null {
  if (session.total_input_tokens == null && session.total_output_tokens == null) return null;
  return (session.total_input_tokens ?? 0) + (session.total_output_tokens ?? 0);
}

export function compareSessions(a: SortableSession, b: SortableSession, key: SessionSortKey, direction: SessionSortDirection): number {
  let comparison = 0;
  switch (key) {
    case 'name': comparison = (a.name ?? a.id).localeCompare(b.name ?? b.id); break;
    case 'status': comparison = a.status.localeCompare(b.status); break;
    case 'uptime': comparison = (a.uptime_secs ?? 0) - (b.uptime_secs ?? 0); break;
    case 'tokens': comparison = (sessionTokens(a) ?? 0) - (sessionTokens(b) ?? 0); break;
    case 'cost': comparison = (a.total_estimated_cost ?? 0) - (b.total_estimated_cost ?? 0); break;
  }
  return direction === 'asc' ? comparison : -comparison;
}
