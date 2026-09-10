import {execFileSync} from 'node:child_process';
import {cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {expect, it} from 'vitest';

it.each([false, true])('clean package build cannot preserve stale files (invalid source: %s)', invalid => {
  const source = fileURLToPath(new URL('../', import.meta.url));
  const fixture = mkdtempSync(join(tmpdir(), 'capsem-sdk-build-'));
  try {
    for (const file of ['package.json', 'tsconfig.json', 'tools']) cpSync(join(source, file), join(fixture, file), {recursive: true});
    symlinkSync(join(source, 'node_modules'), join(fixture, 'node_modules'), 'dir');
    mkdirSync(join(fixture, 'src'));
    writeFileSync(join(fixture, 'src/index.ts'), invalid ? 'export const value: string = 12;' : 'export const value = 12;');
    mkdirSync(join(fixture, 'dist'));
    writeFileSync(join(fixture, 'dist/stale.js'), 'stale');
    const build = (): Buffer => execFileSync(process.execPath, [join(fixture, 'tools/build.mjs')], {timeout: 60_000, stdio: 'pipe'});
    if (invalid) {
      expect(build).toThrow();
      expect(existsSync(join(fixture, 'dist/index.js'))).toBe(false);
    }
    else {
      build();
      expect(readFileSync(join(fixture, 'dist/index.js'), 'utf8')).toContain('export const value = 12');
    }
    expect(existsSync(join(fixture, 'dist/stale.js'))).toBe(false);
  } finally {
    rmSync(fixture, {recursive: true, force: true});
  }
}, 30_000);

it('a clean compiled package exposes the facade and typed operation subpaths', () => {
  const source = fileURLToPath(new URL('../', import.meta.url));
  const fixture = mkdtempSync(join(tmpdir(), 'capsem-sdk-exports-'));
  try {
    for (const file of ['package.json', 'tsconfig.json', 'tools', 'src']) cpSync(join(source, file), join(fixture, file), {recursive: true});
    symlinkSync(join(source, 'node_modules'), join(fixture, 'node_modules'), 'dir');
    expect(existsSync(join(fixture, 'dist'))).toBe(false);
    execFileSync(process.execPath, [join(fixture, 'tools/build.mjs')], {timeout: 60_000, stdio: 'pipe'});
    writeFileSync(join(fixture, 'consumer.ts'), `
      import {Hypervisor, type HypervisorInfo} from '@capsem/sdk';
      import {getHypervisorInfo} from '@capsem/sdk/operations';
      import {Transport} from '@capsem/sdk/transport';
      const high: Promise<HypervisorInfo> = new Hypervisor('http://localhost', 'token').info();
      const low: Promise<HypervisorInfo> = getHypervisorInfo(new Transport('http://localhost', 'token'));
      void high; void low;
    `);
    const compiler = fileURLToPath(import.meta.resolve('typescript/bin/tsc'));
    execFileSync(process.execPath, [compiler, '--noEmit', '--strict', '--target', 'ES2022',
      '--module', 'NodeNext', 'consumer.ts'], {cwd: fixture, timeout: 60_000, stdio: 'pipe'});
    const result = execFileSync(process.execPath, ['--input-type=module', '-e', `
      import {Hypervisor, VM} from '@capsem/sdk';
      import {ServiceAvailability} from '@capsem/sdk/models';
      import {getHypervisorInfo} from '@capsem/sdk/operations';
      import {Transport} from '@capsem/sdk/transport';
      console.log([typeof Hypervisor, typeof VM, ServiceAvailability.RUNNING,
        typeof getHypervisorInfo, typeof Transport].join(','));
    `], {cwd: fixture, timeout: 10_000, encoding: 'utf8'});
    expect(result.trim()).toBe('function,function,running,function,function');
  } finally {
    rmSync(fixture, {recursive: true, force: true});
  }
}, 30_000);
