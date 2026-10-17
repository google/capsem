import {inspect} from 'node:util';
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

// The bootstrap token opens a browser session on the workload. Like the Rust
// and Python SDKs' redacted Debug/repr, logging a Port must not print it.
it('keeps the preview bootstrap token out of serialized and inspected ports', async () => {
  const state = new FacadeGateway();
  await gateway((request, response) => state.handle(request, response), async url => {
    const hv = new Hypervisor(url, 'secret');
    const vm = await hv.create({image: 'nginx:alpine'});
    try {
      const port = await vm.ports.open(3000, {authenticate: true});
      const token = port.bootstrapToken;
      expect(token).toBe('bootstrap-secret');
      for (const text of [JSON.stringify(port), inspect(port, {depth: 5}), String(Object.entries(port))]) {
        expect(text).not.toContain(String(token));
      }
      expect({...port}.url).toBe(port.url);
    } finally {vm.close(); hv.close();}
  });
});
