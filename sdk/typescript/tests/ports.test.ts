import {expect, it} from 'vitest';
import {HttpError, Hypervisor} from '../src/index.js';
import {FacadeGateway} from './facade-gateway.js';
import {gateway} from './gateway.js';

// open({authenticate}) creates the exposure before the session; when the
// session fails the caller has no Port to close, so the SDK must.
it.each([undefined, 500])('closes the exposure when an authenticated port cannot get a session (delete %s)', async deleteStatus => {
  const state = new FacadeGateway();
  state.previewSessionStatus = 503;
  state.exposureDeleteStatus = deleteStatus;
  await gateway((request, response) => state.handle(request, response), async (url, received) => {
    const hv = new Hypervisor(url, 'secret');
    const vm = await hv.create({image: 'nginx:alpine'});
    try {
      const opening = vm.ports.open(3000, {authenticate: true});
      await expect(opening).rejects.toBeInstanceOf(HttpError);
      await expect(opening).rejects.toMatchObject({status: 503});
      expect(received.slice(1).map(request => [request.method, request.url])).toEqual([
        ['POST', '/vms/vm-0/exposures'],
        ['POST', '/vms/vm-0/exposures/preview-id/preview-session'],
        ['DELETE', '/vms/vm-0/exposures/preview-id'],
      ]);
    } finally {vm.close(); hv.close();}
  });
});
