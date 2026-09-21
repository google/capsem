/** One authenticated fetch transport, with bounded requests and explicit cancellation. */
export enum Method {GET = 'GET', POST = 'POST', PUT = 'PUT', DELETE = 'DELETE'}
export enum MediaType {JSON = 'application/json', BINARY = 'application/octet-stream'}

type QueryValue = string | number | boolean | readonly (string | number | boolean)[] | null | undefined;
export interface CallOptions {
  signal?: AbortSignal | undefined;
  /** Replaces the transport's default deadline for this call. */
  timeoutMs?: number | undefined;
}
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

export class NetworkError extends Error {
  override readonly name = 'NetworkError';
  constructor(cause: unknown) {
    super(cause instanceof Error ? cause.message : 'Gateway connection failed', {cause});
  }
}

/**
 * `AbortSignal.any` where it exists; WKWebView before macOS 14.4 (the desktop
 * app supports 14.0) and older runtimes need the linking done by hand.
 */
function checkedTimeout(timeout: number): number {
  if (!Number.isInteger(timeout) || timeout <= 0 || timeout > 2 ** 31 - 1) {
    throw new TypeError('Timeout must be a positive integer no larger than 2147483647 milliseconds');
  }
  return timeout;
}

function anySignal(signals: AbortSignal[]): AbortSignal {
  if (typeof AbortSignal.any === 'function') return AbortSignal.any(signals);
  const linked = new AbortController();
  const unlink = (): void => {for (const signal of signals) signal.removeEventListener('abort', abort);};
  function abort(this: AbortSignal): void {unlink(); linked.abort(this.reason);}
  for (const signal of signals) {
    if (signal.aborted) {linked.abort(signal.reason); return linked.signal;}
  }
  for (const signal of signals) signal.addEventListener('abort', abort, {once: true});
  return linked.signal;
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
    const timeout = checkedTimeout(options.timeoutMs ?? 30_000);
    this.#url = parsed.href.replace(/\/$/, '');
    this.#token = token;
    this.#timeoutMs = timeout;
  }

  /** The default per-request deadline, in milliseconds. */
  get timeoutMs(): number {return this.#timeoutMs;}

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
    const deadline = options.timeoutMs === undefined ? this.#timeoutMs : checkedTimeout(options.timeoutMs);
    const signals = [this.#closed.signal, AbortSignal.timeout(deadline)];
    if (options.signal) signals.push(options.signal);
    const signal = anySignal(signals);
    let response: Response;
    let payload: Uint8Array;
    try {
      response = await fetch(this.#url + path + (query.size ? `?${query}` : ''), {
        method, headers, redirect: 'manual', signal,
        ...(options.body === undefined ? {} : {body: typeof options.body === 'string' ? options.body : new Uint8Array(options.body)}),
      });
      payload = new Uint8Array(await response.arrayBuffer());
    } catch (error) {
      if (signal.aborted) throw error;
      throw new NetworkError(error);
    }
    if (!response.ok) throw new HttpError(response.status, new TextDecoder().decode(payload));
    return payload;
  }
}
