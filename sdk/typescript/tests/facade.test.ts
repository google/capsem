import {expect, it} from 'vitest';
import {HistoryLayerFilter, HostLogSource, HttpError, Hypervisor, RestartAuthentication, RestartStatus, TimelineLayer, VM, type NetworkInfo, type ProfileSummary, type Registry} from '../src/index.js';
import {gateway} from './gateway.js';
import {sample, schemas} from './contract.js';
import {FacadeGateway} from './facade-gateway.js';

it('creates bound VM handles with profile defaults and shared lifetime', async () => {
  await gateway((_, response) => response.end(JSON.stringify({
    ...sample(schemas.ProvisionResponse ?? {}) as object, id: 'vm-0', name: 'chosen',
  })), async (url, received) => {
    const hv = new Hypervisor(url, 'secret');
    const network = {...sample(schemas.NetworkInfo ?? {}) as object, name: 'team'} as NetworkInfo;
    const profile = {...sample(schemas.ProfileSummary ?? {}) as object, id: 'custom'} as ProfileSummary;
    const vm = await hv.create({profile, name: 'chosen', memory: 8, cpus: 4, networks: [network]});
    expect(vm).toBeInstanceOf(VM);
    expect(vm.id).toBe('vm-0');
    expect(vm.name).toBe('chosen');
    expect(JSON.parse(received[0]?.body.toString() ?? '')).toMatchObject({
      profile_id: 'custom', persistent: true, ram_mb: 8192, cpus: 4, networks: ['team'],
    });
    expect(JSON.parse(received[0]?.body.toString() ?? '')).not.toHaveProperty('container');
    vm.close();
    await expect(vm.info()).rejects.toThrow('closed');
    const sibling = await hv.create();
    expect(JSON.parse(received[1]?.body.toString() ?? '')).toMatchObject({persistent: false, cpus: null, ram_mb: null});
    hv.close();
    await expect(sibling.info()).rejects.toThrow('closed');
    await expect(hv.list()).rejects.toThrow('closed');
    sibling.close();
  });
});

it('maps every facade method through HTTP and resolves a name once', async () => {
  const state = new FacadeGateway();
  await gateway((request, response) => state.handle(request, response), async (url, received) => {
    const hv = new Hypervisor(url, 'secret');
    const vm = new VM(url, 'secret', {name: 'chosen'});
    try {
      const bound = hv.vm({id: 'vm-0'});
      expect(bound.id).toBe('vm-0');
      bound.close();
      expect(vm.id).toBeUndefined();
      await vm.info();
      expect(vm.id).toBe('vm-0');
      expect(vm.name).toBe('chosen');
      await vm.exec('uname -a', {timeout_secs: 60});
      await vm.exec('true');
      await vm.start(); await vm.persist('saved'); await vm.pause(); await vm.resume(); await vm.stop();
      await vm.snapshots.list(); await vm.snapshots.status();
      await vm.stats.summary(); await vm.stats.details();
      await vm.history({layer: HistoryLayerFilter.EXEC, limit: 5});
      await vm.history();
      await vm.timeline({layers: [TimelineLayer.FS, TimelineLayer.EXEC]});
      await vm.timeline();
      await vm.log({grep: 'hello', tail: 2, max_bytes: 100});
      await vm.log();
      await vm.files.list(); await vm.files.list('/nested', {depth: 2});
      await vm.files.history('cp-10', {limit: 2}); await vm.files.history('cp-10');
      const fork = await vm.fork('copy', {description: 'checkpoint'});
      expect(fork.id).toBe('fork-0'); expect(fork.name).toBe('copy');
      fork.close();
      const another = await vm.fork('another'); another.close();
      const bytes = new Uint8Array([0, 255]);
      await vm.files.write('/copy.bin', bytes);
      expect(await vm.files.read('/copy.bin')).toEqual(bytes);
      await expect(vm.files.read('/missing')).rejects.toBeInstanceOf(HttpError);
      await vm.delete();
      const expected = [
        '/vms/list', '/vms/vm-0/info', '/vms/vm-0/exec', '/vms/vm-0/exec', '/vms/vm-0/start',
        '/vms/vm-0/save', '/vms/vm-0/pause', '/vms/vm-0/resume', '/vms/vm-0/stop', '/vms/vm-0/snapshots/list',
        '/vms/vm-0/snapshots/status', '/vms/vm-0/stats/summary', '/vms/vm-0/stats/detail',
        '/vms/vm-0/history', '/vms/vm-0/history',
      ];
      expect(received.slice(0, expected.length).map(request => request.url.split('?')[0])).toEqual(expected);
      expect(received.filter(request => request.url === '/vms/list')).toHaveLength(1);
      expect(received.some(request => request.url === '/vms/vm-0/files/list')).toBe(true);
      expect(received.some(request => request.url.includes('layers=fs%2Cexec'))).toBe(true);
      await hv.info(); await hv.list(); await hv.log();
      await hv.log({source: HostLogSource.GATEWAY, tail: 2});
      const [profile] = await hv.profiles.list();
      if (profile === undefined) throw new Error('fixture profile missing');
      await hv.run('printf ok', {profile, timeout_secs: 4});
      await hv.debug.panics({since: '5m', limit: 3});
      await hv.debug.triage({vm_id: 'vm-0', since: '1h', limit: 2});
      await hv.purge({all: true});
      const mcp = hv.profiles.mcp(profile);
      await mcp.info(); await mcp.servers(); await mcp.defaultPermission();
      const server = await mcp.get('local');
      await server.tools.list(); await server.refresh();
      await server.tools.call('read_file', {path: '/tmp/x'});
      await hv.update();
      expect(received.at(-1)?.url).toBe('/update/apply');
      expect(JSON.parse(received.at(-1)?.body.toString() ?? '')).toEqual({confirmed: true});
      const restarted = await hv.restart();
      expect(restarted.status).toBe(RestartStatus.ACCEPTED);
      expect(restarted.authentication).toBe(RestartAuthentication.NEW_TOKEN_REQUIRED);
      expect(received.at(-1)?.method).toBe('POST');
      expect(received.at(-1)?.url).toBe('/restart');
      expect(received.at(-1)?.body.length).toBe(0);
    } finally {
      vm.close(); hv.close();
    }
  });
});

it.each([{names: []}, {names: ['chosen', 'chosen']}])('refuses ambiguous or absent name resolution: $names', async ({names}) => {
  const state = new FacadeGateway(); state.names = names;
  await gateway((request, response) => state.handle(request, response), async (url, received) => {
    const vm = new VM(url, 'secret', {name: 'chosen'});
    try {
      await expect(vm.stop()).rejects.toThrow('Expected one VM');
      expect(received).toHaveLength(1);
      expect(received[0]?.url).toBe('/vms/list');
    } finally {vm.close();}
  });
});

it.each([{}, {id: ''}, {name: ''}, {name: 'a', id: 'b'}, {name: 12}, {id: 12}])('rejects invalid selector %j', selector => {
  expect(() => new VM('http://localhost', 'secret', selector as never)).toThrow('exactly one');
});

it.each([0, -1, NaN, 1.5, '8G', Number.MAX_SAFE_INTEGER])('rejects invalid memory %j before HTTP', async memory => {
  const hv = new Hypervisor('http://127.0.0.1:1', 'secret');
  try {await expect(hv.create({memory: memory as number})).rejects.toThrow('Memory');}
  finally {hv.close();}
});
it.each([0, -1, 1.5])('rejects invalid cpus %s before HTTP', async cpus => {
  const hv = new Hypervisor('http://127.0.0.1:1', 'secret');
  try {await expect(hv.create({cpus})).rejects.toThrow('cpus');}
  finally {hv.close();}
});

it.each([1, 8])('accepts positive memory in GiB: %s', async memory => {
  const state = new FacadeGateway();
  await gateway((request, response) => state.handle(request, response), async (url, received) => {
    const hv = new Hypervisor(url, 'secret');
    try {
      const vm = await hv.create({memory, env: {LANG: 'C'}});
      expect(JSON.parse(received[0]?.body.toString() ?? '')).toMatchObject({ram_mb: memory * 1024, env: {LANG: 'C'}});
      vm.close();
    } finally {hv.close();}
  });
});

it('uses a canonical ID without a name lookup and forwards cancellation', async () => {
  const state = new FacadeGateway();
  await gateway((request, response) => state.handle(request, response), async (url, received) => {
    const vm = new VM(url, 'secret', {id: 'vm-0'});
    try {
      await vm.info();
      expect(received.map(request => request.url)).toEqual(['/vms/vm-0/info']);
      await expect(vm.exec('true', {signal: AbortSignal.abort()})).rejects.toMatchObject({name: 'AbortError'});
      await expect(vm.files.read('/copy.bin', {signal: AbortSignal.abort()})).rejects.toMatchObject({name: 'AbortError'});
      expect(received).toHaveLength(1);
    } finally {vm.close();}
  });
});

it('creates ready containers and exposes read-only diagnostic status', async () => {
  const state = new FacadeGateway();
  await gateway((request, response) => state.handle(request, response), async (url, received) => {
    const hv = new Hypervisor(url, 'secret');
    const registry: Registry = {username: 'robot', password: 'registry-secret'};
    const vm = await hv.create({
      env: {MODE: 'preview'},
      image: 'docker://busybox:latest', command: [], registry,
    });
    try {
      expect(JSON.parse(received[0]?.body.toString() ?? '') as unknown).toMatchObject({
        env: null,
        container: {
          image: 'docker://busybox:latest', env: {MODE: 'preview'},
          registry: {username: 'robot', password: 'registry-secret'}, attach: false,
        },
      });
      expect((await vm.container.status()).image).toBe('docker://busybox:latest');
      expect(received.filter(request => request.url.endsWith('/container')).every(request => request.method === 'GET')).toBe(true);
    } finally {vm.close(); hv.close();}
  });
});

it('rejects container options without an image before HTTP', async () => {
  const hv = new Hypervisor('http://127.0.0.1:1', 'secret');
  try {
    await expect(hv.create({command: ['true']})).rejects.toThrow('require an image');
  } finally {hv.close();}
});

it('opens typed ports and hides exposure targets and preview sessions', async () => {
  const state = new FacadeGateway();
  await gateway((request, response) => state.handle(request, response), async (url, received) => {
    const hv = new Hypervisor(url, 'secret');
    const vm = await hv.create({image: 'nginx:alpine'});
    try {
      const plain = await vm.ports.open(8080);
      const authenticated = await vm.ports.open(3000, {authenticate: true});
      expect(plain.authenticate).toBe(false);
      expect(authenticated.authenticate).toBe(true);
      expect(authenticated.url).toBeDefined();
      await vm.ports.list();
      const requestCount = received.length;
      await expect(vm.ports.close('exp-1' as never)).rejects.toThrow('Port must be an object');
      expect(received).toHaveLength(requestCount);
      await vm.ports.close(plain);
      expect(received.map(request => [request.method, request.url])).toEqual([
        ['POST', '/vms/create'],
        ['POST', '/vms/vm-0/exposures'],
        ['POST', '/vms/vm-0/exposures'],
        ['POST', `/vms/vm-0/exposures/${authenticated.id}/preview-session`],
        ['GET', '/vms/vm-0/exposures'],
        ['DELETE', `/vms/vm-0/exposures/${plain.id}`],
      ]);
    } finally {vm.close(); hv.close();}
  });
});

it('maps the typed network resource including PUT membership and cursor logs', async () => {
  const state = new FacadeGateway();
  await gateway((request, response) => state.handle(request, response), async (url, received) => {
    const hv = new Hypervisor(url, 'secret');
    try {
      const created = await hv.networks.create('team');
      expect((await hv.networks.list()).map(network => network.id)).toEqual(['net-1']);
      await hv.networks.inspect(created.id);
      const vm = hv.vm({id: 'vm-0'});
      expect((await vm.networks.list()).map(network => network.id)).toEqual(['net-1']);
      await vm.networks.attach(created);
      await vm.networks.detach(created);
      await hv.networks.logs(created, {cursor: 'next', limit: 4, type: 'network.connect'});
      await hv.networks.delete(created);
      expect(received.map(request => [request.method, request.url.split('?')[0]])).toEqual([
        ['POST', '/networks'], ['GET', '/networks'], ['GET', `/networks/${created.id}`],
        ['GET', '/networks'],
        ['PUT', `/networks/${created.id}/members/vm-0`],
        ['DELETE', `/networks/${created.id}/members/vm-0`],
        ['GET', `/networks/${created.id}/logs`], ['DELETE', `/networks/${created.id}`],
      ]);
      expect(received.at(-2)?.url).toContain('cursor=next');
      expect(received.at(-2)?.url).toContain('type=network.connect');
      vm.close();
    } finally {hv.close();}
  });
});
