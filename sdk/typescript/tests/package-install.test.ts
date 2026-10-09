import {execFileSync, spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {copyFileSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, renameSync, rmSync, symlinkSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, relative, resolve, sep} from 'node:path';
import {fileURLToPath} from 'node:url';
import {expect, it} from 'vitest';

interface Manifest {name: string; version: string; dependencies: Record<string, string>; devDependencies: Record<string, string>}
interface Report {ok: boolean; sha256: string; version: string; httpPaths: string[]; lifecyclePaths: string[]; credentialPaths: string[]; nodeVersion: string; nodeExecutable: string; nodeArgs: string[]}

const source = fileURLToPath(new URL('../', import.meta.url));
const root = resolve(source, '../..');
const sha = (path: string): string => createHash('sha256').update(readFileSync(path)).digest('hex');

it('prewarms offline without forwarding unsupported pnpm settings to npm', () => {
  const prewarm = spawnSync(process.execPath, [join(source, 'tools/prewarm-package.mjs')], {
    cwd: source, encoding: 'utf8', stdio: 'pipe', timeout: 60_000,
    env: {...process.env, NPM_CONFIG_OFFLINE: 'true',
      npm_config_store_dir: '/unused/pnpm-store', npm_config_verify_deps_before_run: 'false',
      npm_config_overrides: '{}'},
  });
  expect(prewarm.status, prewarm.stdout + prewarm.stderr).toBe(0);
  expect(prewarm.stderr).not.toContain('npm warn');
  const emptyCache = mkdtempSync(join(tmpdir(), 'capsem-sdk-empty-npm-'));
  try {
    const cold = spawnSync(process.execPath, [join(source, 'tools/prewarm-package.mjs')], {
      cwd: source, encoding: 'utf8', stdio: 'pipe', timeout: 60_000,
      env: {...process.env, NPM_CONFIG_OFFLINE: 'true', NPM_CONFIG_CACHE: emptyCache,
        npm_config_store_dir: '/unused/pnpm-store'},
    });
    expect(cold.status).not.toBe(0);
    expect(cold.stderr).toContain('ENOTCACHED');
    expect(cold.stderr).not.toContain('npm warn');
  } finally {
    rmSync(emptyCache, {recursive: true, force: true});
  }
  expect(existsSync(emptyCache)).toBe(false);
}, 65_000);

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
  const consumerNode = process.env.CAPSEM_SDK_PACKAGE_NODE ?? process.execPath;
  const selectedArchive = process.env.CAPSEM_SDK_PACKAGE_ARCHIVE;
  const fixture = mkdtempSync(join(tmpdir(), 'capsem-sdk-package-'));
  expect(relative(root, fixture).startsWith(`..${sep}`)).toBe(true);
  const archiveDirectory = join(fixture, 'archive');
  mkdirSync(archiveDirectory);
  try {
    if (selectedArchive) copyFileSync(selectedArchive, join(archiveDirectory, 'selected-sdk.tgz'));
    else execFileSync('pnpm', ['pack', '--config.ignore-scripts=true', '--pack-destination', archiveDirectory], {
      cwd: source, stdio: 'pipe', timeout: 15_000,
    });
    const archives = readdirSync(archiveDirectory).filter(name => name.endsWith('.tgz'));
    expect(archives).toHaveLength(1);
    const archive = join(archiveDirectory, archives[0]!);
    const digest = sha(archive);
    if (selectedArchive) expect(digest).toBe(sha(selectedArchive));
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
    const runProbe = (): string => execFileSync(consumerNode, [probe, root, archive, output, receipt], {
      cwd: fixture, encoding: 'utf8', stdio: 'pipe', timeout: 15_000,
      env: {PATH: process.env.PATH ?? ''},
    });
    const result = runProbe();
    expect(result).toContain('SDK_IMAGE_PACKAGE_ACCEPTANCE_OK');
    const report = JSON.parse(readFileSync(output, 'utf8')) as Report;
    expect(report.ok).toBe(true);
    expect(report.sha256).toBe(digest);
    expect(report.version).toBe(manifest.version);
    expect(report.nodeVersion).toBe(execFileSync(consumerNode, ['-p', 'process.version'], {encoding: 'utf8', timeout: 5000}).trim());
    expect(realpathSync(report.nodeExecutable)).toBe(realpathSync(consumerNode));
    expect(report.nodeArgs).toEqual([]);
    expect(report.httpPaths).toEqual(['/images?refresh=true', '/images/pull', '/images/pull']);
    expect(report.credentialPaths).toEqual(['/credentials/inject', '/credentials/inject']);
    expect(report.lifecyclePaths).toEqual([
      '/vms/restore-vm/start', '/vms/restore-vm/resume',
      '/vms/restore-vm/start', '/vms/restore-vm/info',
      '/vms/restore-vm/resume', '/vms/restore-vm/info', '/vms/slow/info',
    ]);
    expect(sha(archive)).toBe(digest);
    if (selectedArchive) expect(sha(selectedArchive)).toBe(digest);
    if (process.env.CAPSEM_SDK_PACKAGE_RECEIPT) copyFileSync(output, process.env.CAPSEM_SDK_PACKAGE_RECEIPT);

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
