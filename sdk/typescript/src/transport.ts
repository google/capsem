/** One authenticated fetch transport, with bounded requests and explicit cancellation. */
export enum Method {GET = 'GET', POST = 'POST', DELETE = 'DELETE'}
export enum MediaType {JSON = 'application/json', BINARY = 'application/octet-stream'}

type QueryValue = string | number | boolean | readonly (string | number | boolean)[] | null | undefined;
export interface CallOptions {signal?: AbortSignal | undefined}
export interface TransportOptions {timeoutMs?: number}
interface RequestOptions extends CallOptions {
  parameters?: Record<string, string>;
  query?: Record<string, QueryValue>;
  body?: string | Uint8Array;
  contentType?: MediaType;
  accept?: MediaType;
}

export class HttpError extends Error {
  override readonly name = 'HttpError';
  constructor(readonly status: number, readonly body: string) {
    super(`HTTP ${status}: ${body}`);
  }
}

export class Transport {
  readonly #url: string;
  readonly #token: string;
  readonly #timeoutMs: number;
  readonly #closed = new AbortController();

  constructor(url: string, token: string, options: TransportOptions = {}) {
    const parsed = new URL(url);
    if (!['http:', 'https:'].includes(parsed.protocol) || parsed.username || parsed.password || parsed.search || parsed.hash) {
      throw new TypeError('Gateway URL must be HTTP(S), without credentials, query or fragment');
    }
    if (!token || /[\r\n]/.test(token)) throw new TypeError('Gateway bearer token is required without line breaks');
    const timeout = options.timeoutMs ?? 30_000;
    if (!Number.isInteger(timeout) || timeout <= 0 || timeout > 2 ** 31 - 1) {
      throw new TypeError('Timeout must be a positive integer no larger than 2147483647 milliseconds');
    }
    this.#url = parsed.href.replace(/\/$/, '');
    this.#token = token;
    this.#timeoutMs = timeout;
  }

  close(): void {this.#closed.abort();}

  async request(method: Method, path: string, options: RequestOptions = {}): Promise<Uint8Array> {
    if (this.#closed.signal.aborted) throw new Error('SDK client is closed');
    if (!path.startsWith('/') || /[?#]/.test(path)) throw new TypeError('Operation path must be absolute without query or fragment');
    for (const [name, value] of Object.entries(options.parameters ?? {})) {
      // WHATWG URLs normalize even percent-encoded dot segments before fetch.
      if (!value || value === '.' || value === '..') throw new TypeError('Invalid path identifier');
      path = path.replaceAll(`{${name}}`, encodeURIComponent(value).replaceAll('.', '%2E'));
    }
    if (/[{}]/.test(path)) throw new TypeError('Operation has unresolved path parameters');
    const query = new URLSearchParams();
    for (const [key, value] of Object.entries(options.query ?? {})) {
      if (value !== undefined && value !== null) query.set(key, Array.isArray(value) ? value.join(',') : String(value));
    }
    const headers = new Headers({Authorization: `Bearer ${this.#token}`, Accept: options.accept ?? MediaType.JSON});
    if (options.body !== undefined) headers.set('Content-Type', options.contentType ?? MediaType.JSON);
    const signals = [this.#closed.signal, AbortSignal.timeout(this.#timeoutMs)];
    if (options.signal) signals.push(options.signal);
    const response = await fetch(this.#url + path + (query.size ? `?${query}` : ''), {
      method, headers, redirect: 'manual', signal: AbortSignal.any(signals),
      ...(options.body === undefined ? {} : {body: typeof options.body === 'string' ? options.body : new Uint8Array(options.body)}),
    });
    const payload = new Uint8Array(await response.arrayBuffer());
    if (!response.ok) throw new HttpError(response.status, new TextDecoder().decode(payload));
    return payload;
  }
}
