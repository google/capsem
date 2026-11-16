import {ManagedSessionClaimRequestSchema} from './validation/ManagedSessionClaimRequest.js';

/** The existing managed ownership identity rule, shared by lease and agent clients. */
export const UUID_V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

/** Construct an explicit delegated header value; never discover operator credentials. */
export function delegatedBearer(accessId: string, capability: string): string {
  if (typeof accessId !== 'string' || !UUID_V4.test(accessId)
    || !ManagedSessionClaimRequestSchema.safeParse({ownership_token: capability}).success) {
    throw new TypeError('delegated access requires a canonical access ID and managed capability');
  }
  return `${accessId}.${capability}`;
}
