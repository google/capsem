export interface Config {gatewayUrl: string; token: string; timeoutMs: number}

export function parseConfig(argv: string[]): Config {
  const values = new Map<string, string>();
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index], value = argv[index + 1];
    if (!name?.startsWith('--') || value === undefined) throw new TypeError('gateway URL and token are required');
    if (!['--gateway-url', '--token', '--timeout-ms'].includes(name)) throw new TypeError(`unknown option: ${name}`);
    values.set(name, value);
  }
  const gatewayUrl = values.get('--gateway-url'), token = values.get('--token');
  if (!gatewayUrl || !token) throw new TypeError('gateway URL and token are required');
  const timeoutMs = Number(values.get('--timeout-ms') ?? 30_000);
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0) throw new TypeError('timeout must be a positive integer');
  return {gatewayUrl, token, timeoutMs};
}
