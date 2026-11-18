import {expect, it} from 'vitest';
import {Hypervisor, type ProvisionRequest} from '../src/index.js';
import {gateway} from './gateway.js';
import {sample, schemas} from './contract.js';

for (const managed of [false, true]) {
  for (const [hasImage, attached, inline] of [[true, false, false], [false, false, false], [true, true, false], [true, false, true]] as const) {
    it(`discovers saved environment port target once: managed=${String(managed)} image=${String(hasImage)} attached=${String(attached)} inline=${String(inline)}`, async () => {
      let requestId = '';
      const ownership = (): object => ({request_id: requestId, generation: '22222222-2222-4222-8222-222222222222',
        expires_at: '2099-01-01T00:00:00Z', lease_seconds: 4, renew_after_seconds: 1});
      await gateway((request, response) => {
        expect(request.headers.authorization).toBe('Bearer token');
        if (request.url.endsWith('/readiness')) {
          const parts = request.url.split('/');
          response.end(JSON.stringify({environment: {environment_id: parts[2], revision_id: parts[4]}, startup_timeout_seconds: hasImage ? 600 : 0}));
        } else if (request.url === '/vms/create') {
          const body = JSON.parse(request.body.toString()) as ProvisionRequest;
          requestId = body.managed?.request_id ?? '';
          response.end(JSON.stringify({...sample(schemas.ProvisionResponse ?? {}) as object,
            id: 'vm-1', name: 'vm-1', persistent: false, ownership: requestId ? ownership() : null}));
        } else if (request.url.endsWith('/claim')) {
          response.end(JSON.stringify({ownership: ownership(), state: 'active'}));
        } else if (request.url.endsWith('/close')) {
          response.end(JSON.stringify({request_id: requestId, generation: '22222222-2222-4222-8222-222222222222',
            state: 'closed', session_id: 'vm-1', evidence_retained: false}));
        } else if (request.url.endsWith('/container')) {
          if (!hasImage) response.writeHead(404).end('no container');
          else response.end(JSON.stringify(sample(schemas.ContainerStatusResponse ?? {})));
        } else {
          expect(request.url).toBe('/vms/vm-1/exposures');
          response.end(JSON.stringify({...sample(schemas.ExposureInfo ?? {}) as object,
            ...JSON.parse(request.body.toString()) as object, id: 'port', host_port: 49152}));
        }
      }, async (url, received) => {
        const hv = new Hypervisor(url, 'token');
        const environment = {environment_id: '00000000-0000-4000-8000-000000000001', revision_id: '00000000-0000-4000-8000-000000000002'};
        const options = inline ? {image: 'busybox'} : {environment, attach: attached};
        if (managed) {
          await hv.withManaged(options, async vm => {
            await vm.ports.open(8080);
            await vm.ports.open(8081);
          });
        } else {
          const vm = await hv.create(options);
          await vm.ports.open(8080);
          await vm.ports.open(8081);
          vm.close();
        }
        expect(received.filter(request => request.url.endsWith('/readiness'))).toHaveLength(attached || inline ? 0 : 1);
        expect(received.filter(request => request.url.endsWith('/container'))).toHaveLength(attached || inline ? 0 : 1);
        const exposures = received.filter(request => request.url.endsWith('/exposures'));
        expect(exposures).toHaveLength(2);
        for (const request of exposures) expect((JSON.parse(request.body.toString()) as {target: string}).target).toBe(hasImage ? 'container' : 'vm');
        hv.close();
      });
    });
  }
}

for (const managed of [false, true]) {
  for (const outcome of ['changed', 'unavailable', 'overflow']) {
    it(`refuses ${outcome} timing before creation or managed ownership: managed=${String(managed)}`, async () => {
      let requestId = '';
      const ownership = (): object => ({request_id: requestId, generation: '22222222-2222-4222-8222-222222222222',
        expires_at: '2099-01-01T00:00:00Z', lease_seconds: 4, renew_after_seconds: 1});
      await gateway((request, response) => {
        if (request.url.endsWith('/readiness')) {
          if (outcome === 'unavailable') response.writeHead(403).end('inspection refused');
          else response.end(JSON.stringify({environment: {environment_id: '00000000-0000-4000-8000-000000000001', revision_id: outcome === 'changed' ? '00000000-0000-4000-8000-000000000009' : '00000000-0000-4000-8000-000000000002'}, startup_timeout_seconds: outcome === 'overflow' ? 2 ** 32 : 600}));
        } else if (request.url === '/vms/create') {
          requestId = (JSON.parse(request.body.toString()) as ProvisionRequest).managed?.request_id ?? '';
          response.end(JSON.stringify({...sample(schemas.ProvisionResponse ?? {}) as object,
            id: 'vm-1', name: 'vm-1', persistent: false, ownership: requestId ? ownership() : null}));
        } else if (request.url.endsWith('/claim')) response.end(JSON.stringify({ownership: ownership(), state: 'active'}));
        else response.end(JSON.stringify({request_id: requestId, generation: '22222222-2222-4222-8222-222222222222', state: 'closed', session_id: 'vm-1', evidence_retained: false}));
      }, async (url, received) => {
        const hv = new Hypervisor(url, 'token');
        const environment = {environment_id: '00000000-0000-4000-8000-000000000001', revision_id: '00000000-0000-4000-8000-000000000002'};
        try {
          const result = managed ? hv.withManaged({environment}, () => Promise.reject(new Error('unexpected callback'))) : hv.create({environment});
          await expect(result).rejects.toThrow(outcome === 'changed' ? /identity/ : outcome === 'unavailable' ? /403/ : /startup timeout/);
          expect(received).toHaveLength(1);
          expect(received[0]?.url).toContain('/readiness');
        } finally {hv.close();}
      });
    });
  }
}

it('cancels the timing read before managed ownership exists', async () => {
  let entered!: () => void;
  const arrived = new Promise<void>(resolve => {entered = resolve;});
  await gateway(request => {
    expect(request.url).toContain('/readiness');
    entered();
  }, async (url, received) => {
    const hv = new Hypervisor(url, 'token');
    const controller = new AbortController();
    const reason = new Error('cancel before creation');
    const environment = {environment_id: '00000000-0000-4000-8000-000000000001', revision_id: '00000000-0000-4000-8000-000000000002'};
    try {
      const opening = hv.managed({environment, signal: controller.signal});
      const rejected = expect(opening).rejects.toBe(reason);
      await arrived;
      controller.abort(reason);
      await rejected;
      expect(received).toHaveLength(1);
      expect(received[0]?.url).toContain('/readiness');
    } finally {controller.abort(); hv.close();}
  });
});
