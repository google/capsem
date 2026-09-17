import {expect, it} from 'vitest';
import {Hypervisor} from '../src/index.js';
import {FacadeGateway} from './facade-gateway.js';
import {gateway} from './gateway.js';

// The gateway's catalog names the default profile; the SDK must not carry a
// profile name of its own.
it('creates and runs with the catalog default, resolved once', async () => {
  const state = new FacadeGateway();
  state.defaultProfileId = 'co-work';
  await gateway((request, response) => state.handle(request, response), async (url, received) => {
    const hv = new Hypervisor(url, 'secret');
    try {
      await hv.create();
      await hv.run('true');
      const bodies = received.filter(r => ['/vms/create', '/run'].includes(r.url))
        .map(r => JSON.parse(r.body.toString()) as {profile_id: string});
      expect(bodies.map(body => body.profile_id)).toEqual(['co-work', 'co-work']);
      expect(received.filter(r => r.url === '/status')).toHaveLength(1);
    } finally {hv.close();}
  });
});

it('never asks for the default when a profile is named', async () => {
  const state = new FacadeGateway();
  await gateway((request, response) => state.handle(request, response), async (url, received) => {
    const hv = new Hypervisor(url, 'secret');
    try {
      const [profile] = await hv.profiles.list();
      if (!profile) throw new Error('fixture profile');
      await hv.create({profile});
      expect(received.map(r => r.url)).not.toContain('/status');
    } finally {hv.close();}
  });
});

it('reports a catalog without a default instead of inventing one', async () => {
  const state = new FacadeGateway();
  state.defaultProfileId = undefined;
  await gateway((request, response) => state.handle(request, response), async (url, received) => {
    const hv = new Hypervisor(url, 'secret');
    try {
      await expect(hv.create()).rejects.toThrow(/names no default profile/);
      expect(received.map(r => r.url)).not.toContain('/vms/create');
    } finally {hv.close();}
  });
});
