/** Explicit managed capability fixtures; no operator runtime or discovery. */
export const accessId = 'fc120782-2272-4b3d-a2e6-a26ee2f8ab61';
export const capability = 'af'.repeat(32);
export const delegated = {accessId, capability};
export const authorization = (secret = capability): string => `Bearer ${accessId}.${secret}`;
