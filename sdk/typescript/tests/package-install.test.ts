import {execFileSync, spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {copyFileSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, renameSync, rmSync, symlinkSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, relative, resolve, sep} from 'node:path';
import {fileURLToPath} from 'node:url';
import {expect, it} from 'vitest';

interface Manifest {name: string; version: string; dependencies: Record<string, string>; devDependencies: Record<string, string>}
interface Report {ok: boolean; sha256: string; version: string; httpPaths: string[]}

const source = fileURLToPath(new URL('../', import.meta.url));
const root = resolve(source, '../..');
const sha = (path: string): string => createHash('sha256').update(readFileSync(path)).digest('hex');

function inventory(directory: string, base = directory): Record<string, string> {
  const files: Record<string, string> = {};
  for (const name of readdirSync(directory)) {
    const path = join(directory, name), stat = lstatSync(path);
    expect(stat.isSymbolicLink()).toBe(false);
    if (stat.isDirectory()) Object.assign(files, inventory(path, base));
    else {
      expect(stat.isFile()).toBe(true);
      files[relative(base, path)] = sha(path);
    }
  }
  return files;
}

it('installs and exercises the actual SDK tarball without checkout or development dependencies', () => {
  const manifest = JSON.parse(readFileSync(join(source, 'package.json'), 'utf8')) as Manifest;
  const fixture = mkdtempSync(join(tmpdir(), 'capsem-sdk-package-'));
  expect(relative(root, fixture).startsWith(`..${sep}`)).toBe(true);
  const archiveDirectory = join(fixture, 'archive');
  mkdirSync(archiveDirectory);
  try {
    execFileSync('pnpm', ['pack', '--config.ignore-scripts=true', '--pack-destination', archiveDirectory], {
      cwd: source, stdio: 'pipe', timeout: 15_000,
    });
    const archives = readdirSync(archiveDirectory).filter(name => name.endsWith('.tgz'));
    expect(archives).toHaveLength(1);
    const archive = join(archiveDirectory, archives[0]!);
    const digest = sha(archive);
    execFileSync('tar', ['-xzf', archive, '-C', archiveDirectory], {timeout: 5000});
    const files = inventory(join(archiveDirectory, 'package'));
    const receipt = join(fixture, 'payload.json');
    writeFileSync(receipt, JSON.stringify({archiveSha256: digest, files}));
    writeFileSync(join(fixture, 'package.json'), JSON.stringify({
      name: 'capsem-sdk-clean-consumer', private: true,
    }));

    // The gate prewarms npm before entering its sandbox. This is an ordinary
    // tarball install into a fresh runtime-only, non-linked consumer.
    const install = spawnSync('npm', ['install', '--offline', '--omit=dev', '--ignore-scripts',
      '--no-audit', '--no-fund', archive], {
      cwd: fixture, encoding: 'utf8', stdio: 'pipe', timeout: 60_000,
    });
    expect(install.status, install.stdout + install.stderr).toBe(0);
    const modules = join(fixture, 'node_modules');
    for (const dependency of Object.keys(manifest.devDependencies)) {
      expect(existsSync(join(modules, dependency))).toBe(false);
    }
    expect(inventory(join(modules, manifest.name))).toEqual(files);
    const probe = join(fixture, 'acceptance.mjs');
    const output = join(fixture, 'acceptance.json');
    copyFileSync(join(source, 'tools/image-package-acceptance.mjs'), probe);
    const runProbe = (): string => execFileSync(process.execPath, [probe, root, archive, output, receipt], {
      cwd: fixture, encoding: 'utf8', stdio: 'pipe', timeout: 15_000,
      env: {PATH: process.env.PATH ?? ''},
    });
    const result = runProbe();
    expect(result).toContain('SDK_IMAGE_PACKAGE_ACCEPTANCE_OK');
    const report = JSON.parse(readFileSync(output, 'utf8')) as Report;
    expect(report.ok).toBe(true);
    expect(report.sha256).toBe(digest);
    expect(report.version).toBe(manifest.version);
    expect(report.httpPaths).toEqual(['/images?refresh=true', '/images/pull', '/images/pull']);
    expect(sha(archive)).toBe(digest);

    const entrypoint = join(modules, manifest.name, 'dist/index.js');
    const original = readFileSync(entrypoint);
    writeFileSync(entrypoint, Buffer.concat([original, Buffer.from('\n// altered installed payload\n')]));
    expect(runProbe).toThrow();
    writeFileSync(entrypoint, original);

    const packageRoot = join(modules, manifest.name), moved = join(fixture, 'linked-sdk');
    renameSync(packageRoot, moved);
    symlinkSync(moved, packageRoot, 'dir');
    expect(runProbe).toThrow();
    rmSync(packageRoot);
    renameSync(moved, packageRoot);

    // Install a real compiler from the prewarmed cache to prove the probe
    // refuses development-tool contamination, without faking its inventory.
    execFileSync('npm', ['install', '--offline', '--ignore-scripts', '--no-audit', '--no-fund',
      `typescript@${manifest.devDependencies.typescript}`], {
      cwd: fixture, stdio: 'pipe', timeout: 30_000,
    });
    expect(existsSync(join(modules, 'typescript/package.json'))).toBe(true);
    expect(runProbe).toThrow(/development dependency/);
  } finally {
    rmSync(fixture, {recursive: true, force: true});
  }
  expect(existsSync(fixture)).toBe(false);
}, 90_000);
