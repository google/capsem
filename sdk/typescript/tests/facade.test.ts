import {expect, it} from 'vitest';
import {HistoryLayerFilter, HostLogSource, HttpError, Hypervisor, TimelineLayer, VM} from '../src/index.js';
import {gateway} from './gateway.js';
import {sample, schemas} from './contract.js';
import {FacadeGateway} from './facade-gateway.js';

it('creates bound VM handles with profile defaults and shared lifetime', async () => {
  await gateway((_, response) => response.end(JSON.stringify({
    ...sample(schemas.ProvisionResponse ?? {}) as object, id: 'vm-0', name: 'chosen',
  })), async (url, received) => {
    const hv = new Hypervisor(url, 'secret');
    const vm = await hv.create('code', {name: 'chosen', memory: '8G', vcpu: 4});
    expect(vm).toBeInstanceOf(VM);
    expect(vm.id).toBe('vm-0');
    expect(vm.name).toBe('chosen');
    expect(JSON.parse(received[0]?.body.toString() ?? '')).toMatchObject({
      profile_id: 'code', persistent: true, ram_mb: 8192, cpus: 4,
    });
    vm.close();
    await expect(vm.info()).rejects.toThrow('closed');
    const sibling = await hv.create('code');
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
      expect(vm.id).toBeUndefined();
      await vm.info();
      expect(vm.id).toBe('vm-0');
      expect(vm.name).toBe('chosen');
      await vm.exec('uname -a', {timeout_secs: 60});
      await vm.exec('true');
      await vm.start(); await vm.pause(); await vm.resume(); await vm.stop();
      await vm.snapshots.list(); await vm.snapshots.status();
      await vm.stats.summary(); await vm.stats.details();
      await vm.history({layer: HistoryLayerFilter.EXEC, limit: 5});
      await vm.history();
      await vm.timeline({layers: [TimelineLayer.FS, TimelineLayer.EXEC]});
      await vm.timeline();
      await vm.log({grep: 'hello', tail: 2, max_bytes: 100});
      await vm.log();
      await vm.list(); await vm.list('/nested', {depth: 2});
      await vm.changes('cp-10', {limit: 2}); await vm.changes('cp-10');
      const fork = await vm.fork('copy', {description: 'checkpoint'});
      expect(fork.id).toBe('fork-0'); expect(fork.name).toBe('copy');
      fork.close();
      const another = await vm.fork('another'); another.close();
      const bytes = new Uint8Array([0, 255]);
      await vm.copy.toVm('/copy.bin', bytes);
      expect(await vm.copy.fromVm('/copy.bin')).toEqual(bytes);
      await expect(vm.copy.fromVm('/missing')).rejects.toBeInstanceOf(HttpError);
      await vm.delete();
      const expected = [
        '/vms/list', '/vms/vm-0/info', '/vms/vm-0/exec', '/vms/vm-0/exec', '/vms/vm-0/start',
        '/vms/vm-0/pause', '/vms/vm-0/resume', '/vms/vm-0/stop', '/vms/vm-0/snapshots/list',
        '/vms/vm-0/snapshots/status', '/vms/vm-0/stats/summary', '/vms/vm-0/stats/detail',
        '/vms/vm-0/history', '/vms/vm-0/history',
      ];
      expect(received.slice(0, expected.length).map(request => request.url.split('?')[0])).toEqual(expected);
      expect(received.filter(request => request.url === '/vms/list')).toHaveLength(1);
      expect(received.some(request => request.url === '/vms/vm-0/files/list')).toBe(true);
      expect(received.some(request => request.url.includes('layers=fs%2Cexec'))).toBe(true);
      await hv.info(); await hv.list(); await hv.log();
      await hv.log({source: HostLogSource.GATEWAY, tail: 2});
      await hv.update();
      expect(received.at(-1)?.url).toBe('/update/apply');
      expect(JSON.parse(received.at(-1)?.body.toString() ?? '')).toEqual({confirmed: true});
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

it.each([0, -1, NaN, 1.5, '8GB', '0G', '1.5G', '9007199254740992M'])('rejects invalid memory %j before HTTP', async memory => {
  const hv = new Hypervisor('http://127.0.0.1:1', 'secret');
  try {await expect(hv.create('code', {memory})).rejects.toThrow('Memory');}
  finally {hv.close();}
});
it.each([0, -1, 1.5])('rejects invalid vcpu %s before HTTP', async vcpu => {
  const hv = new Hypervisor('http://127.0.0.1:1', 'secret');
  try {await expect(hv.create('code', {vcpu})).rejects.toThrow('vcpu');}
  finally {hv.close();}
});

it.each(['512M', 512])('accepts positive memory in MB: %s', async memory => {
  const state = new FacadeGateway();
  await gateway((request, response) => state.handle(request, response), async (url, received) => {
    const hv = new Hypervisor(url, 'secret');
    try {
      const vm = await hv.create('code', {memory, env: {LANG: 'C'}});
      expect(JSON.parse(received[0]?.body.toString() ?? '')).toMatchObject({ram_mb: 512, env: {LANG: 'C'}});
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
      await expect(vm.copy.fromVm('/copy.bin', {signal: AbortSignal.abort()})).rejects.toMatchObject({name: 'AbortError'});
      expect(received).toHaveLength(1);
    } finally {vm.close();}
  });
});
