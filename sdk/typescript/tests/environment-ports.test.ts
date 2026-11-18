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
        if (request.url === '/vms/create') {
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
        expect(received.filter(request => request.url.endsWith('/container'))).toHaveLength(attached || inline ? 0 : 1);
        const exposures = received.filter(request => request.url.endsWith('/exposures'));
        expect(exposures).toHaveLength(2);
        for (const request of exposures) expect((JSON.parse(request.body.toString()) as {target: string}).target).toBe(hasImage ? 'container' : 'vm');
        hv.close();
      });
    });
  }
}
