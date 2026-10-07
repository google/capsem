import {expect, it} from 'vitest';
import {ExecTarget, HttpError, VM} from '../src/index.js';
import {gateway} from './gateway.js';

const result = {stdout: {encoding: 'utf8', data: 'target'}, stderr: {encoding: 'utf8', data: ''}, exit_code: 0};

it('sends explicit targets and preserves the default authenticated exec body', async () => {
  await gateway((_request, response) => response.end(JSON.stringify(result)), async (url, received) => {
    const vm = new VM(url, 'token', {id: 'immutable-id'});
    try {
      for (const target of [ExecTarget.VM, ExecTarget.WORKLOAD]) {
        expect(await vm.exec('printf target', {timeout_secs: 12, target})).toEqual(result);
        expect(JSON.parse(received.at(-1)?.body.toString() ?? '')).toEqual({command: 'printf target', timeout_secs: 12, target});
      }
      await vm.exec('printf default', {timeout_secs: 12});
      expect(JSON.parse(received.at(-1)?.body.toString() ?? '')).toEqual({command: 'printf default', timeout_secs: 12});
      await vm.exec('printf default', {timeout_secs: 12, target: null});
      expect(JSON.parse(received.at(-1)?.body.toString() ?? '')).not.toHaveProperty('target');
      expect(received).toHaveLength(4);
      for (const request of received) {
        expect([request.method, request.url]).toEqual(['POST', '/vms/immutable-id/exec']);
        expect(request.headers.authorization).toBe('Bearer token');
      }
    } finally {vm.close();}
  });
});

it('rejects malformed targets before resolving a display name or dispatching HTTP', async () => {
  await gateway((_request, response) => response.end('{}'), async (url, received) => {
    const vm = new VM(url, 'token', {name: 'unresolved'});
    try {
      for (const target of ['root', '', 1, []]) {
        await expect(vm.exec('printf no', {target: target as unknown as ExecTarget})).rejects.toThrow();
      }
      expect(received).toHaveLength(0);
      expect(vm.id).toBeUndefined();
    } finally {vm.close();}
  });
});

it('keeps service target refusal without VM fallback or replay', async () => {
  await gateway((_request, response) => response.writeHead(409).end('target unavailable'), async (url, received) => {
    const vm = new VM(url, 'token', {id: 'immutable-id'});
    try {
      const pending = vm.exec('printf no', {target: ExecTarget.WORKLOAD});
      await expect(pending).rejects.toBeInstanceOf(HttpError);
      await expect(pending).rejects.toMatchObject({status: 409});
      expect(received).toHaveLength(1);
      expect(JSON.parse(received[0]?.body.toString() ?? '')).toMatchObject({target: 'workload'});
    } finally {vm.close();}
  });
});

it('closes an in-flight target request when its caller cancels', async () => {
  let markStarted!: () => void;
  let markClosed!: () => void;
  const started = new Promise<void>(resolve => {markStarted = resolve;});
  const closed = new Promise<void>(resolve => {markClosed = resolve;});
  await gateway((_request, response) => {
    response.once('close', markClosed);
    markStarted();
  }, async (url, received) => {
    const vm = new VM(url, 'token', {id: 'immutable-id'});
    const controller = new AbortController();
    try {
      const pending = vm.exec('sleep 99', {target: ExecTarget.VM, signal: controller.signal});
      await started;
      controller.abort();
      await expect(pending).rejects.toThrow();
      let timer: ReturnType<typeof setTimeout> | undefined;
      try {
        await Promise.race([closed, new Promise<never>((_resolve, reject) => {
          timer = setTimeout(() => reject(new Error('cancelled exec retained its HTTP request')), 500);
        })]);
      } finally {clearTimeout(timer);}
      expect(received).toHaveLength(1);
      expect(JSON.parse(received[0]?.body.toString() ?? '')).toMatchObject({target: 'vm'});
    } finally {vm.close();}
  });
});

it('does not dispatch an explicitly targeted exec after caller cancellation', async () => {
  await gateway((_request, response) => response.end(JSON.stringify(result)), async (url, received) => {
    const vm = new VM(url, 'token', {id: 'immutable-id'});
    const controller = new AbortController();
    controller.abort();
    try {
      await expect(vm.exec('sleep 99', {target: ExecTarget.VM, signal: controller.signal})).rejects.toThrow();
      expect(received).toHaveLength(0);
    } finally {vm.close();}
  });
});
