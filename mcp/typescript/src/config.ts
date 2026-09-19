import {readFileSync} from 'node:fs';

export interface Config {gatewayUrl: string; token: string; timeoutMs: number}

/** The environment variable carrying the gateway bearer token. */
export const TOKEN_ENV = 'CAPSEM_GATEWAY_TOKEN';
const OPTIONS = ['--gateway-url', '--token-file', '--timeout-ms'];

/**
 * Settings from argv, the token from `--token-file` or `CAPSEM_GATEWAY_TOKEN`.
 * The token never travels on argv, where `ps` and `/proc/<pid>/cmdline` expose
 * it for the server's lifetime. Errors are TypeErrors that name options, never
 * their values, so the CLI may print them.
 */
export function parseConfig(argv: string[], env: NodeJS.ProcessEnv = process.env): Config {
  const values = new Map<string, string>();
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index], value = argv[index + 1];
    if (name === '--token' || name?.startsWith('--token=')) {
      throw new TypeError(`--token would expose the gateway token in the process list; set ${TOKEN_ENV} or pass --token-file`);
    }
    if (!name?.startsWith('--') || value === undefined) throw new TypeError('gateway URL and token are required');
    if (!OPTIONS.includes(name)) throw new TypeError(`unknown option: ${name}`);
    values.set(name, value);
  }
  const gatewayUrl = values.get('--gateway-url');
  const tokenFile = values.get('--token-file');
  const token = tokenFile === undefined ? env[TOKEN_ENV] : readToken(tokenFile);
  if (!gatewayUrl || !token) throw new TypeError(`gateway URL and token are required (--gateway-url; ${TOKEN_ENV} or --token-file)`);
  const timeoutMs = Number(values.get('--timeout-ms') ?? 30_000);
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0) throw new TypeError('timeout must be a positive integer');
  return {gatewayUrl, token, timeoutMs};
}

function readToken(path: string): string {
  let token: string;
  try {
    token = readFileSync(path, 'utf8').trim();
  } catch {
    throw new TypeError('cannot read the token file named by --token-file');
  }
  if (!token) throw new TypeError('the token file named by --token-file is empty');
  return token;
}
