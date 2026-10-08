// Run after an ordinary package install outside the checkout.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {execFileSync} from 'node:child_process';
import {once} from 'node:events';
import {createServer} from 'node:http';
import {existsSync, readFileSync, lstatSync, readdirSync, writeFileSync} from 'node:fs';
import {dirname, join, relative, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {buffer} from 'node:stream/consumers';
import {setTimeout as delay} from 'node:timers/promises';
import process from 'node:process';

const packageName = '@capsem/sdk';
/** @type {unknown} */
const built = await import(packageName);
const {VM, Hypervisor, ImageCacheState, VmLifecycleState} = /** @type {typeof import('../src/index.js')} */ (built);

const [sourceRoot, archive, output, payloadReceipt] = process.argv.slice(2);
assert.ok(sourceRoot && archive && output && payloadReceipt);
const source = resolve(sourceRoot);
const project = resolve(dirname(fileURLToPath(import.meta.url)));
assert.ok(!project.startsWith(resolve(sourceRoot) + '/'));
const packageRoot = join(project, 'node_modules/@capsem/sdk');
assert.ok(!lstatSync(packageRoot).isSymbolicLink());
/** @typedef {{name:string,version:string,engines:Record<string,string>,dependencies:Record<string,string>,devDependencies:Record<string,string>,exports:Record<string,{types:string,import:string}>,scripts:Record<string,string>}} Manifest */
/** @param {string} path @returns {unknown} */
function readJson(path) { return JSON.parse(readFileSync(path, 'utf8')); }
const manifest = /** @type {Manifest} */ (readJson(join(packageRoot, 'package.json')));
const sourceManifest = /** @type {Manifest} */ (readJson(join(source, 'sdk/typescript/package.json')));
// This local pack omits the project's prepack build hook from its manifest.
const packedManifest = {...sourceManifest, scripts: {...sourceManifest.scripts}};
delete packedManifest.scripts.prepack;
assert.deepEqual(manifest, packedManifest);
for (const name of Object.keys(manifest.devDependencies)) {
  assert.ok(!existsSync(join(project, 'node_modules', name)), `runtime contains development dependency: ${name}`);
}
/** @type {Record<string,string>} */
const origins = {};
// Node 20.0 needs a flag for this tooling-only resolver. Keep the consumer
// and its actual SDK operations unflagged; delegate only origin inspection.
/** @param {string} name @returns {string} */
function resolveInstalled(name) {
  if (typeof import.meta.resolve === 'function') return import.meta.resolve(name);
  const resolver = join(project, 'resolve-installed.mjs');
  writeFileSync(resolver, 'process.stdout.write(import.meta.resolve(process.argv[2]));\n');
  return execFileSync(process.execPath, ['--experimental-import-meta-resolve',
    resolver, name], {
    cwd: project, encoding: 'utf8', timeout: 5000, env: {PATH: process.env.PATH ?? ''},
  });
}
for (const entry of Object.keys(manifest.exports)) {
  const name = entry === '.' ? '@capsem/sdk' : '@capsem/sdk' + entry.slice(1);
  const origin = fileURLToPath(resolveInstalled(name));
  assert.ok(origin.startsWith(packageRoot + '/'));
  await import(name);
  origins[name] = origin;
  const exported = manifest.exports[entry];
  assert.ok(exported);
  assert.ok(readFileSync(join(packageRoot, exported.types)).length > 0);
}
assert.ok(fileURLToPath(resolveInstalled('zod')).startsWith(join(project, 'node_modules') + '/'));
/** @type {Record<string,string>} */
const hashes = {};
/** @param {string} directory */
function inventory(directory) {
  for (const name of readdirSync(directory)) {
    const path = join(directory, name), stat = lstatSync(path);
    assert.ok(!stat.isSymbolicLink());
    if (stat.isDirectory()) inventory(path);
    else {
      const key = relative(packageRoot, path);
      const bytes = readFileSync(path);
      if (key !== 'package.json') assert.deepEqual(bytes, readFileSync(join(source, 'sdk/typescript', key)));
      hashes[key] = createHash('sha256').update(bytes).digest('hex');
    }
  }
}
inventory(packageRoot);
const receipt = /** @type {{archiveSha256:string,files:Record<string,string>}} */ (readJson(payloadReceipt));
assert.equal(receipt.archiveSha256, createHash('sha256').update(readFileSync(archive)).digest('hex'));
assert.deepEqual(hashes, receipt.files);
const pin = `registry.example/code@sha256:${'a'.repeat(64)}`;
/** @type {{method:string|undefined,path:string|undefined,body:unknown}[]} */
const received = [];
let restoreEntered = () => {};
const provision = {id: 'restore-vm', name: 'restore', status: 'Running', available_actions: []};
const server = createServer((request, response) => {
  void buffer(request).then(async bytes => {
    assert.equal(request.headers.authorization, 'Bearer fixture-token');
    received.push({method: request.method, path: request.url, body: bytes.length ? /** @type {unknown} */ (JSON.parse(bytes.toString())) : null});
    if (request.url?.startsWith('/vms/')) {
      if (request.url.endsWith('/start') || request.url.endsWith('/resume')) {
        restoreEntered();
        await delay(300);
      } else if (request.url === '/vms/slow/info') {
        await delay(300);
      }
      response.setHeader('Content-Type', 'application/json');
      response.end(JSON.stringify(request.method === 'GET' ? {...provision, pid: 1} : provision));
      return;
    }
    response.setHeader('Content-Type', 'application/json');
    response.end(JSON.stringify(request.method === 'GET'
      ? {images: [{name: 'code', description: 'Tools', architectures: ['amd64'], image: pin, cached: 'unknown'}]}
      : {image: 'code', resolved: pin, digest: `sha256:${'b'.repeat(64)}`}));
  }).catch((/** @type {unknown} */ error) => response.destroy(error instanceof Error ? error : new Error(String(error))));
});
server.listen(0, '127.0.0.1');
await once(server, 'listening');
const address = server.address();
assert.ok(address && typeof address !== 'string');
const hv = new Hypervisor(`http://127.0.0.1:${address.port}`, 'fixture-token');
try {
  const catalog = await hv.images.list({refresh: true});
  assert.equal(catalog.images[0]?.image, pin);
  assert.equal(catalog.images[0]?.cached, ImageCacheState.UNKNOWN);
  assert.equal((await hv.images.pull('code', {registry: {username: 'fixture', password: 'fixture-access'}})).resolved, pin);
  await hv.images.pull('code');
  const images = hv.images;
  hv.close();
  await assert.rejects(images.list(), /closed/);
  const vm = new VM(`http://127.0.0.1:${address.port}`, 'fixture-token', {id: 'restore-vm'}, {timeoutMs: 50});
  const slow = new VM(`http://127.0.0.1:${address.port}`, 'fixture-token', {id: 'slow'}, {timeoutMs: 50});
  try {
    for (const operation of ['start', 'resume']) {
      const result = await vm[/** @type {'start'|'resume'} */ (operation)]();
      assert.equal(result.id, 'restore-vm');
      assert.equal(result.status, VmLifecycleState.RUNNING);
    }
    for (const operation of ['start', 'resume']) {
      const controller = new AbortController();
      const entered = new Promise(resolve => {restoreEntered = () => {resolve(undefined);};});
      const pending = vm[/** @type {'start'|'resume'} */ (operation)]({signal: controller.signal});
      const cancelled = assert.rejects(pending, {name: 'AbortError'});
      try {
        await entered;
        controller.abort();
        await cancelled;
        const info = await vm.info();
        assert.equal(info.id, 'restore-vm');
        assert.equal(info.pid, 1);
      } finally {controller.abort(); await cancelled;}
    }
    await assert.rejects(slow.info(), {name: 'TimeoutError'});
  } finally {vm.close(); slow.close();}
} finally {
  hv.close();
  server.closeAllConnections();
  await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve(undefined)));
}
const imageRequests = received.filter(request => request.path?.startsWith('/images'));
const lifecycleRequests = received.filter(request => request.path?.startsWith('/vms/'));
assert.equal(imageRequests.length + lifecycleRequests.length, received.length);
assert.deepEqual(imageRequests.map(({method, path}) => [method, path]), [['GET', '/images?refresh=true'], ['POST', '/images/pull'], ['POST', '/images/pull']]);
assert.deepEqual(lifecycleRequests, [
  {method: 'POST', path: '/vms/restore-vm/start', body: null},
  {method: 'POST', path: '/vms/restore-vm/resume', body: null},
  {method: 'POST', path: '/vms/restore-vm/start', body: null},
  {method: 'GET', path: '/vms/restore-vm/info', body: null},
  {method: 'POST', path: '/vms/restore-vm/resume', body: null},
  {method: 'GET', path: '/vms/restore-vm/info', body: null},
  {method: 'GET', path: '/vms/slow/info', body: null},
]);
assert.deepEqual(received[1]?.body, {image: 'code', registry: {username: 'fixture', password: 'fixture-access'}});
assert.deepEqual(received[2]?.body, {image: 'code'});
writeFileSync(output, JSON.stringify({archive, sha256: createHash('sha256').update(readFileSync(archive)).digest('hex'), version: manifest.version, engines: manifest.engines, dependencies: manifest.dependencies, origins, payloadFiles: Object.keys(hashes).length, payloadHashes: hashes, httpPaths: imageRequests.map(request => request.path), lifecyclePaths: lifecycleRequests.map(request => request.path), nodeVersion: process.version, nodeExecutable: process.execPath, nodeArgs: process.execArgv, ok: true}, null, 2) + '\n');
process.stdout.write('SDK_IMAGE_PACKAGE_ACCEPTANCE_OK\n');
