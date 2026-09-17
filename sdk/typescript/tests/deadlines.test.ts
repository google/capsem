import {expect, it} from 'vitest';
import {commandDeadlineMs} from '../src/execution.js';
import {Hypervisor, VM} from '../src/index.js';
import {FacadeGateway} from './facade-gateway.js';
import {gateway} from './gateway.js';

// Exec and run answer only when the command ends (service default: one hour).
// A 30 s client deadline gave up while the command kept running, and a
// retrying agent started it a second time.
it.each([
  [30_000, undefined, (3600 + 120) * 1000],
  [30_000, 600, (600 + 120) * 1000],
  [5_000_000, 10, 5_000_000],
])('deadline for default %i ms and timeout_secs %s is %i ms', (fallback, timeoutSecs, expected) => {
  expect(commandDeadlineMs(fallback, timeoutSecs)).toBe(expected);
});

function delayed(state: FacadeGateway, paths: string[]): Parameters<typeof gateway>[0] {
  return (request, response) => {
    const wait = paths.includes(new URL(request.url, 'http://localhost').pathname) ? 300 : 0;
    setTimeout(() => {state.handle(request, response);}, wait);
  };
}

it('lets exec and run outlive the default deadline without replaying them', async () => {
  const state = new FacadeGateway();
  await gateway(delayed(state, ['/vms/vm-0/exec', '/run']), async (url, received) => {
    const vm = new VM(url, 'secret', {id: 'vm-0'}, {timeoutMs: 50});
    const hv = new Hypervisor(url, 'secret', {timeoutMs: 50});
    try {
      await vm.exec('slow build', {timeout_secs: 600});
      await hv.run('slow build');
      expect(received.map(request => request.url)).toEqual(['/vms/vm-0/exec', '/run']);
    } finally {vm.close(); hv.close();}
  });
});

it('keeps the default deadline for ordinary calls and honours an explicit one', async () => {
  const state = new FacadeGateway();
  await gateway(delayed(state, ['/vms/vm-0/info', '/vms/vm-0/exec']), async (url, received) => {
    const vm = new VM(url, 'secret', {id: 'vm-0'}, {timeoutMs: 50});
    try {
      await expect(vm.info()).rejects.toMatchObject({name: 'TimeoutError'});
      await expect(vm.exec('bounded', {timeoutMs: 50})).rejects.toMatchObject({name: 'TimeoutError'});
      expect(received).toHaveLength(2);
    } finally {vm.close();}
  });
});
