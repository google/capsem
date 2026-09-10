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
