import {expect, it} from 'vitest';
import {Hypervisor} from '../src/index.js';
import {gateway} from './gateway.js';
import {sample, schemas} from './contract.js';

it.each([false, true])('binds the clone source and preserves the destination override (%s)', async override => {
  const saved = {environment_id: '00000000-0000-4000-8000-000000000001', revision_id: '00000000-0000-4000-8000-000000000002'};
  const destination = {...saved, revision_id: '00000000-0000-4000-8000-000000000004'};
  const selection = {id: 'vm-0', running: false, generation: '00000000-0000-4000-8000-000000000003', environment: saved};
  const sourcePath = '/vms/source-name/readiness?intent=clone';
  const destinationPath = `/environments/${destination.environment_id}/revisions/${destination.revision_id}/readiness`;
  await gateway((request, reply) => {
    switch (request.url) {
      case sourcePath:
        reply.end(JSON.stringify({selection, startup_timeout_seconds: 600}));
        break;
      case destinationPath:
        reply.end(JSON.stringify({environment: destination, startup_timeout_seconds: 20}));
        break;
      default:
        reply.end(JSON.stringify({...sample(schemas.ProvisionResponse ?? {}) as object, id: 'clone-0', name: 'clone'}));
    }
  }, async (url, received) => {
    const hv = new Hypervisor(url, 'secret');
    try {
      const vm = await hv.create({from: 'source-name', ...(override ? {environment: destination} : {})});
      expect(vm.id).toBe('clone-0');
      expect(received.map(request => request.url)).toEqual(override ? [sourcePath, destinationPath, '/vms/create'] : [sourcePath, '/vms/create']);
      const body = JSON.parse(received.at(-1)?.body.toString() ?? '') as Record<string, unknown>;
      expect(body.from).toBe('source-name');
      expect(body.expected_source).toEqual(selection);
      expect(body.environment).toEqual(override ? destination : undefined);
      vm.close();
    } finally {hv.close();}
  });
});

it('preserves the exact saved revision and refuses inline overrides before HTTP', async () => {
  const environment = {environment_id: '00000000-0000-4000-8000-000000000001', revision_id: '00000000-0000-4000-8000-000000000002'};
  await gateway((request, response) => response.end(JSON.stringify(request.url.endsWith('/readiness')
    ? {environment, startup_timeout_seconds: 600}
    : {...sample(schemas.ProvisionResponse ?? {}) as object, id: 'vm-0', name: 'chosen'})),
  async (url, received) => {
    const hv = new Hypervisor(url, 'secret');
    const vm = await hv.create({environment});
    const body = JSON.parse(received.filter(request => request.url === '/vms/create')[0]?.body.toString() ?? '') as {environment?: unknown};
    expect(body.environment).toEqual(environment);
    expect(body).not.toHaveProperty('attach');
    const attached = await hv.create({environment, attach: true});
    const attachedBody = JSON.parse(received.filter(request => request.url === '/vms/create')[1]?.body.toString() ?? '') as Record<string, unknown>;
    expect(attachedBody.environment).toEqual(environment);
    expect(attachedBody.attach).toBe(true);
    expect(attachedBody).not.toHaveProperty('container');
    expect(attachedBody.env).toBeNull();
    await expect(hv.create({attach: true})).rejects.toThrow('environment');
    await expect(hv.create({environment, env: {}})).rejects.toThrow('inline');
    await expect(hv.create({environment, image: 'busybox'})).rejects.toThrow('inline');
    expect(received).toHaveLength(3);
    expect(received[0]?.url).toContain('/readiness');
    attached.close();
    vm.close();
    hv.close();
  });
});
