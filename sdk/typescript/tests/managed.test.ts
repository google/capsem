import type {ServerResponse} from 'node:http';
import {expect, it} from 'vitest';
import {Hypervisor, type ProvisionRequest, type ManagedSessionOwnership} from '../src/index.js';
import {gateway, type Request} from './gateway.js';

const GENERATION = '46a98f98-4683-4b09-b319-e235179d76bb';
const VM_ID = '2b17c25b-7993-42f8-b16b-60c10dc8c60e';

function deferred(): {promise: Promise<void>; resolve: () => void} {
  let resolve!: () => void;
  const promise = new Promise<void>(done => {resolve = done;});
  return {promise, resolve};
}

function untilAborted(signal: AbortSignal): Promise<void> {
  return new Promise((_resolve, reject) => signal.addEventListener('abort', () => {
    const reason: unknown = signal.reason;
    reject(reason instanceof Error ? reason : new Error('Cancelled', {cause: reason}));
  }, {once: true}));
}

class Service {
  readonly created = deferred();
  readonly claimed = deferred();
  readonly closing = deferred();
  readonly releaseCreate = deferred();
  readonly releaseClose = deferred();
  readonly releaseClaim = deferred();
  request: ProvisionRequest | undefined;
  state = 'pending';
  holdCreate = false;
  holdClose = false;
  holdClaim = false;
  failures = 0;
  missing = 0;
  invalidReply = false;
  generation = GENERATION;
  expiryMs = 4000;

  ownership(generation = GENERATION): ManagedSessionOwnership {
    return {request_id: this.request?.managed?.request_id ?? '', generation,
      lease_seconds: 4, renew_after_seconds: 1, expires_at: new Date(Date.now() + this.expiryMs).toISOString()};
  }
  session(): object {
    return {id: VM_ID, name: VM_ID, status: 'Running', persistent: false, can_resume: false,
      available_actions: ['stop', 'delete'], ownership: this.ownership()};
  }
  handle = (request: Request, response: ServerResponse): void => {
    void this.respond(request, response);
  };
  private async respond(request: Request, response: ServerResponse): Promise<void> {
    expect(request.method).toBe('POST');
    expect(request.headers.authorization).toBe('Bearer gateway-bearer');
    const body = JSON.parse(request.body.toString()) as ProvisionRequest & {ownership_token?: string};
    if (request.url === '/vms/create') {
      expect(this.request).toBeUndefined();
      this.request = body;
      this.created.resolve();
      if (this.holdCreate) await this.releaseCreate.promise;
      if (this.state === 'closed') {response.writeHead(409).end('{}'); return;}
      this.state = 'active';
      response.end(this.invalidReply ? 'lost-create-response' : JSON.stringify(this.session()));
      return;
    }
    expect(body.ownership_token).toBe(this.request?.managed?.ownership_token);
    expect(request.url).toBe(`/vms/managed/${this.request?.managed?.request_id ?? ''}/${request.url.split('/').at(-1) ?? ''}`);
    if (request.url.endsWith('/claim')) {
      if (this.missing-- > 0) {response.writeHead(404).end('{"code":"not_found","error":"pending admission"}'); return;}
      this.claimed.resolve();
      if (this.holdClaim) await this.releaseClaim.promise;
      response.end(JSON.stringify({ownership: this.ownership(this.generation), state: this.state,
        session: this.state === 'active' ? this.session() : null}));
      return;
    }
    expect(request.url.endsWith('/close')).toBe(true);
    this.closing.resolve();
    if (this.holdClose) await this.releaseClose.promise;
    if (this.failures-- > 0) {response.writeHead(500).end('{"code":"cleanup_failed","error":"cleanup pending"}'); return;}
    this.state = 'closed';
    this.releaseClaim.resolve();
    this.releaseCreate.resolve();
    response.end(JSON.stringify({request_id: this.request?.managed?.request_id, generation: GENERATION,
      state: 'closed', session_id: VM_ID, evidence_retained: false}));
  }
}

it.each([{expiryMs: 200, holdClaim: false}, {expiryMs: 1200, holdClaim: true}])('expires an owned callback even during a stalled renewal ($holdClaim)', async ({expiryMs, holdClaim}) => {
  const service = new Service();
  service.expiryMs = expiryMs;
  service.holdClaim = holdClaim;
  await gateway(service.handle, async url => {
    const hv = new Hypervisor(url, 'gateway-bearer');
    const scope = await hv.managed({});
    let timer: ReturnType<typeof setTimeout> | undefined;
    const deadline = new Promise<never>((_resolve, reject) => {timer = setTimeout(() => {reject(new Error('Missed lease expiry'));}, expiryMs + 550);});
    try {
      await expect(Promise.race([scope.run(async (_vm, signal) => {await untilAborted(signal);}), deadline])).rejects.toThrow('expired');
      expect(service.state).toBe('closed');
    } finally {
      clearTimeout(timer);
      await scope.close();
      hv.close();
    }
  });
});

it('renews finite service ownership and keeps ordinary handle close connection-only', async () => {
  const service = new Service();
  await gateway(service.handle, async (url, requests) => {
    const hv = new Hypervisor(url, 'gateway-bearer');
    const scope = await hv.managed({image: 'docker://busybox', env: {MODE: 'owned'}});
    await scope.run(async vm => {
      expect(vm.id).toBe(VM_ID);
      expect(JSON.stringify(scope)).not.toContain(service.request?.managed?.ownership_token);
      await service.claimed.promise;
      hv.vm({name: 'unrelated-named-session'}).close();
      expect(service.state).toBe('active');
    });
    expect(service.state).toBe('closed');
    const token = service.request?.managed?.ownership_token ?? '';
    expect(token).toMatch(/^[0-9a-f]{64}$/);
    expect(service.request?.managed?.request_id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    expect(JSON.stringify(scope)).not.toContain(token);
    expect(requests.filter(request => request.url === '/vms/create')).toHaveLength(1);
    expect(requests.every(request => !request.url.includes(token) && !JSON.stringify(request.headers).includes(token))).toBe(true);
    expect(service.request?.persistent).toBe(false);
    expect(service.request?.name).toBeNull();
    expect(service.request?.container?.env).toEqual({MODE: 'owned'});
    expect(service.request?.env).toBeNull();
    hv.close();
  });
});

it.each([false, true])('awaits cleanup after caller abort (pending create: %s)', async pending => {
  const service = new Service();
  service.holdCreate = pending;
  service.holdClose = true;
  const abort = new AbortController();
  const reason = new Error('caller cancelled');
  await gateway(service.handle, async (url, requests) => {
    const hv = new Hypervisor(url, 'gateway-bearer');
    const entered = deferred();
    let finished = false;
    const result = hv.withManaged({signal: abort.signal}, async (_vm, signal) => {
      entered.resolve();
      await untilAborted(signal);
    }).finally(() => {finished = true;});
    const rejected = expect(result).rejects.toBe(reason);
    await (pending ? service.created.promise : entered.promise);
    abort.abort(reason);
    await service.closing.promise;
    expect(finished).toBe(false);
    service.releaseClose.resolve();
    await rejected;
    expect(service.state).toBe('closed');
    expect(requests.filter(request => request.url === '/vms/create')).toHaveLength(1);
    expect(JSON.parse(requests.at(-1)?.body.toString() ?? '{}')).toMatchObject({outcome: 'cancelled'});
    hv.close();
  });
});

it('refuses a changed ownership generation and closes before returning the failure', async () => {
  const service = new Service();
  service.generation = '67652132-695e-432a-ac42-0e0fb995ddfd';
  await gateway(service.handle, async url => {
    const hv = new Hypervisor(url, 'gateway-bearer');
    await expect(hv.withManaged({}, async (_vm, signal) => {
      await untilAborted(signal);
    })).rejects.toThrow('ownership');
    expect(service.state).toBe('closed');
    hv.close();
  });
});

it('reports cleanup failure and retries only an explicit close on the original owner', async () => {
  const service = new Service();
  service.failures = 1;
  await gateway(service.handle, async (url, requests) => {
    const hv = new Hypervisor(url, 'gateway-bearer');
    const scope = await hv.managed();
    await expect(scope.run(() => Promise.resolve('done'))).rejects.toMatchObject({status: 500});
    expect(service.state).toBe('active');
    await scope.close();
    expect(service.state).toBe('closed');
    expect(requests.filter(request => request.url === '/vms/create')).toHaveLength(1);
    expect(requests.filter(request => request.url.endsWith('/close'))).toHaveLength(2);
    hv.close();
  });
});

it('claims pending ownership through admission races without replaying create', async () => {
  const service = new Service();
  service.holdCreate = true;
  service.missing = 1;
  await gateway(service.handle, async (url, requests) => {
    const hv = new Hypervisor(url, 'gateway-bearer');
    const result = hv.withManaged({}, vm => Promise.resolve(vm.id));
    await service.claimed.promise;
    service.releaseCreate.resolve();
    expect(await result).toBe(VM_ID);
    expect(service.state).toBe('closed');
    expect(requests.filter(request => request.url === '/vms/create')).toHaveLength(1);
    expect(requests.filter(request => request.url.endsWith('/claim')).length).toBeGreaterThanOrEqual(2);
    hv.close();
  });
});

it('closes an uncertain create response before exposing any VM handle', async () => {
  const service = new Service();
  service.invalidReply = true;
  await gateway(service.handle, async (url, requests) => {
    const hv = new Hypervisor(url, 'gateway-bearer');
    let bodyEntered = false;
    await expect(hv.withManaged({}, () => {bodyEntered = true; return Promise.resolve();})).rejects.toThrow();
    expect(bodyEntered).toBe(false);
    expect(service.state).toBe('closed');
    expect(requests.filter(request => request.url === '/vms/create')).toHaveLength(1);
    hv.close();
  });
});

it('refuses an already-aborted caller before any create or cleanup mutation', async () => {
  const abort = new AbortController();
  const reason = new Error('already cancelled');
  abort.abort(reason);
  await gateway((_request, response) => response.end('{}'), async (url, requests) => {
    const hv = new Hypervisor(url, 'gateway-bearer');
    await expect(hv.managed({signal: abort.signal})).rejects.toBe(reason);
    expect(requests).toHaveLength(0);
    hv.close();
  });
});
