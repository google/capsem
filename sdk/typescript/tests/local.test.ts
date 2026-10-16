import {chmodSync, mkdtempSync, mkdirSync, renameSync, rmSync, symlinkSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {afterEach, beforeEach, expect, it, vi} from 'vitest';
import {Hypervisor} from '../src/index.js';
import {FacadeGateway} from './facade-gateway.js';
import {gateway} from './gateway.js';

let root: string;
beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), 'capsem-sdk-local-'));
  vi.stubEnv('CAPSEM_RUN_DIR', root);
});
afterEach(() => {vi.unstubAllEnvs(); rmSync(root, {recursive: true, force: true});});

function metadata(directory = root, port = '19222', token = 'secret'): void {
  mkdirSync(directory, {recursive: true});
  writeFileSync(join(directory, 'gateway.port'), port);
  writeFileSync(join(directory, 'gateway.token'), token, {mode: 0o600});
}

it('connects with no arguments and sends the discovered token only to loopback', async () => {
  const state = new FacadeGateway();
  await gateway((request, response) => state.handle(request, response), async (url, received) => {
    metadata(root, new URL(url).port);
    const hv = new Hypervisor();
    try {await hv.list();} finally {hv.close();}
    expect(received).toHaveLength(1);
    expect(received[0]?.url).toBe('/vms/list');
    expect(received[0]?.headers.authorization).toBe('Bearer secret');
  });
});

it.each(['HOME', 'CAPSEM_HOME', 'CAPSEM_RUN_DIR'])('uses %s with the documented precedence', key => {
  vi.stubEnv('CAPSEM_RUN_DIR', '');
  vi.stubEnv('CAPSEM_HOME', '');
  vi.stubEnv('HOME', join(root, 'missing'));
  vi.stubEnv(key, root);
  metadata(key === 'HOME' ? join(root, '.capsem', 'run') : key === 'CAPSEM_HOME' ? join(root, 'run') : root);
  new Hypervisor().close();
});

it.each(['0', '65536', '-1', '1.5', '19222/evil', 'evil.example:80', '1\n2', '', '1'.repeat(5000)])('refuses invalid local port %s', port => {
  metadata(root, port);
  expect(() => new Hypervisor()).toThrow(/local Capsem/);
});

it.each(['', 'a\nb', 'a\rb', 'x'.repeat(5000)])('refuses invalid tokens without including their contents', token => {
  metadata(root, '19222', token);
  expect(() => new Hypervisor()).toThrow(/local Capsem/);
  try {new Hypervisor();} catch (error) {
    if (token) expect(String(error)).not.toContain(token);
  }
});

it('refuses missing, public or symlinked credentials', () => {
  expect(() => new Hypervisor()).toThrow(/Start Capsem/);
  metadata();
  const token = join(root, 'gateway.token');
  chmodSync(token, 0o644);
  expect(() => new Hypervisor()).toThrow();
  renameSync(token, join(root, 'secret'));
  symlinkSync(join(root, 'secret'), token);
  expect(() => new Hypervisor()).toThrow();
});

it('never fills partial remote arguments from local credentials', () => {
  expect(() => new Hypervisor('https://remote.example')).toThrow(/both/);
  expect(() => new Hypervisor(undefined, 'remote-token')).toThrow(/both/);
  new Hypervisor('https://remote.example', 'remote-token').close();
});

it('reads rotated credentials for each new connection', async () => {
  const state = new FacadeGateway();
  await gateway((request, response) => state.handle(request, response), async (url, received) => {
    metadata(root, new URL(url).port);
    const first = new Hypervisor();
    metadata(root, new URL(url).port, 'rotated');
    const second = new Hypervisor();
    try {await first.list(); await second.list();} finally {first.close(); second.close();}
    expect(received.map(request => request.headers.authorization)).toEqual(['Bearer secret', 'Bearer rotated']);
  });
});
