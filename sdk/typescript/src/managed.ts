import * as api from './operations/index.js';
import * as models from './models/index.js';
import {createDeadlineMs} from './execution.js';
import {HttpError, Transport, type CallOptions} from './transport.js';
import {VM} from './vm.js';

const UUID_V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

function wait(ms: number, signal: AbortSignal, wake: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(done, ms);
    function unlink(): void {clearTimeout(timer); signal.removeEventListener('abort', abort); wake.removeEventListener('abort', done);}
    function done(): void {unlink(); resolve();}
    function abort(): void {
      unlink();
      const reason: unknown = signal.reason;
      reject(reason instanceof Error ? reason : new Error('Managed request cancelled', {cause: reason}));
    }
    if (signal.aborted) abort();
    else if (wake.aborted) done();
    else {signal.addEventListener('abort', abort, {once: true}); wake.addEventListener('abort', done, {once: true});}
  });
}

/** Finite ownership of one ephemeral VM; ordinary VM.close stays connection-only. */
export class ManagedSession {
  readonly #transport: Transport;
  readonly #request: models.ProvisionRequest;
  readonly #options: CallOptions;
  readonly #requestId = globalThis.crypto.randomUUID();
  #token = Array.from(globalThis.crypto.getRandomValues(new Uint8Array(32)), byte => byte.toString(16).padStart(2, '0')).join('');
  readonly #requests = new AbortController();
  readonly #lifetime = new AbortController();
  #wake: AbortController | undefined;
  #generation: string | undefined;
  #ownership: models.ManagedSessionOwnership | undefined;
  #vm: VM | undefined;
  #vmId: string | undefined;
  #creation: Promise<models.ProvisionResponse> | undefined;
  #creationDone = false;
  #renewal: Promise<void> | undefined;
  #closing: Promise<void> | undefined;
  #closeFailed = false;
  #finished = false;
  #ran = false;
  #failed = false;
  #failure: unknown;
  #reject!: (reason: unknown) => void;
  readonly #ended = new Promise<never>((_resolve, reject) => {this.#reject = reject;});
  readonly #abort = (): void => this.#fail(this.#options.signal?.reason, models.ManagedSessionOutcome.CANCELLED);

  private constructor(transport: Transport, request: models.ProvisionRequest, options: CallOptions) {
    this.#transport = transport;
    this.#request = request;
    this.#options = {signal: options.signal, timeoutMs: options.timeoutMs};
    if (request.name || request.persistent || request.managed) throw new TypeError('Managed sessions require an unnamed ephemeral create');
    request.managed = {request_id: this.#requestId, ownership_token: this.#token};
    // A failure may arrive before run() starts; retain it without an unhandled rejection.
    void this.#ended.catch(() => {});
  }

  /** @internal */
  static async open(transport: Transport, request: models.ProvisionRequest, options: CallOptions): Promise<ManagedSession> {
    const scope = new ManagedSession(transport, request, options);
    if (options.signal?.aborted) throw options.signal.reason;
    options.signal?.addEventListener('abort', scope.#abort, {once: true});
    scope.#creation = api.createVm(transport, {body: request}, {
      signal: scope.#requests.signal, timeoutMs: options.timeoutMs ?? createDeadlineMs(transport.timeoutMs),
    }).finally(() => {scope.#creationDone = true;});
    scope.#renewal = scope.#maintain().catch((error: unknown) => {
      if (!scope.#closing) scope.#fail(error, models.ManagedSessionOutcome.FAILED);
    });
    try {
      const response = await Promise.race([scope.#creation, scope.#ended]);
      scope.#session(response);
      if (scope.#failed) throw scope.#failure;
      scope.#vm = VM.bind(transport, response.id, response.name, request.container != null);
      return scope;
    } catch (error) {
      await scope.#join(options.signal?.aborted ? models.ManagedSessionOutcome.CANCELLED : models.ManagedSessionOutcome.FAILED, false);
      throw error;
    }
  }

  get ownership(): models.ManagedSessionOwnership | undefined {return this.#ownership && {...this.#ownership};}
  get vm(): VM {
    if (!this.#vm || this.#finished || this.#closing) throw new Error('Managed session is closed');
    return this.#vm;
  }
  /** Aborts when the owner is cancelled, closes, or loses its lease. */
  get signal(): AbortSignal {return this.#lifetime.signal;}

  /** Await the original close before returning the callback's result or failure. */
  async run<T>(work: (vm: VM, signal: AbortSignal) => Promise<T>): Promise<T> {
    if (this.#ran) throw new Error('Managed ownership scope cannot run again');
    this.#ran = true;
    try {
      if (this.#failed) throw this.#failure;
      const result = await Promise.race([Promise.resolve().then(() => work(this.vm, this.signal)), this.#ended]);
      await this.#join(models.ManagedSessionOutcome.COMPLETED, false);
      if (this.#failed) throw this.#failure;
      return result;
    } catch (error) {
      await this.#join(this.#options.signal?.aborted ? models.ManagedSessionOutcome.CANCELLED : models.ManagedSessionOutcome.FAILED, false);
      throw error;
    }
  }

  /** Join cleanup; a failed close is retried only by another explicit close call. */
  async close(outcome = models.ManagedSessionOutcome.COMPLETED): Promise<void> {await this.#join(outcome, true);}

  #identity(requestId: string, generation: string): void {
    if (requestId !== this.#requestId || !UUID_V4.test(generation)
      || (this.#generation !== undefined && this.#generation !== generation)) throw new Error('Managed ownership changed');
    this.#generation = generation;
  }
  #adopt(ownership: models.ManagedSessionOwnership): void {
    this.#identity(ownership.request_id, ownership.generation);
    const expiry = Date.parse(ownership.expires_at);
    if (!Number.isSafeInteger(ownership.lease_seconds) || !Number.isSafeInteger(ownership.renew_after_seconds)
      || ownership.renew_after_seconds <= 0 || ownership.renew_after_seconds >= ownership.lease_seconds
      || !Number.isFinite(expiry) || expiry <= Date.now()) throw new Error('Invalid or expired managed ownership lease');
    this.#ownership = {...ownership};
    this.#wake?.abort();
  }
  #session(response: models.ProvisionResponse): void {
    if (!response.ownership || response.persistent || !response.id) throw new Error('Managed session omitted its ephemeral ownership');
    this.#adopt(response.ownership);
    if (this.#vmId !== undefined && this.#vmId !== response.id) throw new Error('Managed ownership changed VM identity');
    this.#vmId = response.id;
  }
  #fail(reason: unknown, outcome: models.ManagedSessionOutcome): void {
    if (this.#finished || this.#failed) return;
    this.#failed = true;
    this.#failure = reason;
    this.#lifetime.abort(reason);
    this.#reject(reason);
    if (!this.#closing) void this.#join(outcome, false).catch(() => {});
  }
  async #maintain(): Promise<void> {
    while (true) {
      const ownership = this.#ownership;
      const delay = ownership === undefined ? 50 : Math.min(2 ** 31 - 1, ownership.renew_after_seconds * 1000);
      const wake = this.#wake = new AbortController();
      await wait(delay, this.#requests.signal, wake.signal);
      if (wake.signal.aborted) continue;
      let claim: models.ManagedSessionClaimResponse;
      try {
        claim = await api.claimManagedSession(this.#transport, {request_id: this.#requestId,
          body: {ownership_token: this.#token}}, {signal: this.#requests.signal});
      } catch (error) {
        if (error instanceof HttpError && error.status === 404 && this.#generation === undefined && !this.#creationDone) continue;
        throw error;
      }
      if (claim.state !== models.ManagedSessionState.PENDING && claim.state !== models.ManagedSessionState.ACTIVE) {
        throw new Error('Managed ownership is no longer live');
      }
      this.#adopt(claim.ownership);
      if (claim.session) this.#session(claim.session);
    }
  }
  async #join(outcome: models.ManagedSessionOutcome, retry: boolean): Promise<void> {
    if (this.#finished) return;
    if (!this.#closing || (retry && this.#closeFailed)) {
      this.#closeFailed = false;
      this.#closing = this.#cleanup(outcome).catch((error: unknown) => {this.#closeFailed = true; throw error;});
      void this.#closing.catch(() => {});
    }
    await this.#closing;
  }
  async #cleanup(outcome: models.ManagedSessionOutcome): Promise<void> {
    this.#requests.abort();
    this.#lifetime.abort();
    this.#vm?.close();
    await Promise.allSettled([this.#creation, this.#renewal]);
    try {
      const closed = await api.closeManagedSession(this.#transport, {request_id: this.#requestId,
        body: {ownership_token: this.#token, outcome}}, {timeoutMs: createDeadlineMs(this.#transport.timeoutMs)});
      this.#identity(closed.request_id, closed.generation);
      if (this.#vmId !== undefined && closed.session_id !== this.#vmId) throw new Error('Managed close acknowledged another VM identity');
      this.#finished = true;
      this.#token = '';
      delete this.#request.managed;
    } finally {this.#options.signal?.removeEventListener('abort', this.#abort);}
  }
}
