import {expect, it} from 'vitest';
import {Hypervisor} from '../src/index.js';
import {gateway} from './gateway.js';
import {sample, schemas} from './contract.js';

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
