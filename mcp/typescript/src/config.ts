import {delegatedBearer} from '@capsem/sdk';

export interface Config {gatewayUrl: string; accessId: string; capability: string; timeoutMs: number}

/** The existing managed capability is secret; its access ID is not. */
export const CAPABILITY_ENV = 'CAPSEM_AGENT_CAPABILITY';
const OPTIONS = ['--gateway-url', '--access-id', '--timeout-ms'];

/** Explicit delegated settings only. Never read operator files or environment. */
export function parseConfig(argv: string[], env: NodeJS.ProcessEnv = process.env): Config {
  const values = new Map<string, string>();
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index], value = argv[index + 1];
    if (name?.startsWith('--token') || name?.startsWith('--capability')) {
      throw new TypeError(`delegated capabilities must use ${CAPABILITY_ENV}; token files and argv secrets are unsupported`);
    }
    if (!name?.startsWith('--') || value === undefined) throw new TypeError('explicit gateway URL and delegated access ID are required');
    if (!OPTIONS.includes(name)) throw new TypeError('unknown host MCP option');
    if (values.has(name)) throw new TypeError('duplicate host MCP option');
    values.set(name, value);
  }
  const gatewayUrl = values.get('--gateway-url');
  const accessId = values.get('--access-id');
  const capability = env[CAPABILITY_ENV];
  if (!gatewayUrl || !accessId || !capability) {
    throw new TypeError(`explicit gateway URL, delegated access ID and ${CAPABILITY_ENV} capability are required`);
  }
  delegatedBearer(accessId, capability);
  const timeoutMs = Number(values.get('--timeout-ms') ?? 30_000);
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0) throw new TypeError('timeout must be a positive integer');
  return {gatewayUrl, accessId, capability, timeoutMs};
}
