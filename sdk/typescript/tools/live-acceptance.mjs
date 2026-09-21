import assert from 'node:assert/strict';
import {createHash, randomUUID} from 'node:crypto';
import process from 'node:process';

const packageName = '@capsem/sdk';
/** @type {unknown} */
const built = await import(packageName);
const {Hypervisor, VmLifecycleState} = /** @type {typeof import('../src/index.js')} */ (built);
const url = process.env.SDK_GATEWAY_URL, token = process.env.SDK_GATEWAY_TOKEN;
assert(url && token, 'Fixture gateway URL and token are required');
/**
 * @param {import('../src/index.js').ExecOutput} output
 * @param {string} expected
 */
function assertOutput(output, expected) {
  assert.equal(output.encoding, 'utf8');
  assert.equal(output.data, expected);
}
const hv = new Hypervisor(url, token, {timeoutMs: 120_000});
const name = `sdk-ts-${randomUUID().slice(0, 8)}`;
try {
  const requestedProfile = process.env.CAPSEM_TEST_PROFILE;
  const profile = requestedProfile === undefined
    ? undefined
    : (await hv.profiles.list()).find(item => item.id === requestedProfile);
  assert(requestedProfile === undefined || profile !== undefined, `missing profile ${requestedProfile}`);
  const vm = await hv.create({name, cpus: 2, memory: 2, ...(profile === undefined ? {} : {profile})});
  assertOutput((await vm.exec('printf SDK_EXEC_READY')).stdout, 'SDK_EXEC_READY');
  const info = await vm.info();
  assert.equal(info.id, vm.id);
  assert.equal(info.status, VmLifecycleState.RUNNING);
  assert(info.persistent && info.ai && info.network && info.files);
  const data = Uint8Array.from({length: 4364}, (_, index) => index % 256);
  const upload = await vm.files.write('sdk-proof.bin', data);
  assert(upload.success && upload.size === data.length);
  assert.deepEqual(await vm.files.read('sdk-proof.bin'), data);
  assert((await vm.files.list()).entries.some(entry => entry.name === 'sdk-proof.bin'));
  const digest = createHash('sha256').update(data).digest('hex');
  const exec = await vm.exec('sha256sum /root/sdk-proof.bin; printf SDK_STDERR >&2; exit 7');
  assert.equal(exec.exit_code, 7);
  assert.equal(exec.stdout.data.split(' ')[0], digest);
  assertOutput(exec.stderr, 'SDK_STDERR');
  await vm.log({tail: 10});
  await hv.log({tail: 10});
  await vm.history({limit: 10});
  await vm.stats.summary();
  await vm.stats.details();
  await vm.snapshots.list();
  await vm.snapshots.status();
  const stopped = await vm.stop();
  assert(stopped.success && stopped.persistent);
  assert.equal((await vm.info()).status, VmLifecycleState.STOPPED);
  const fork = await vm.fork(`${name}-fork`);
  assert.notEqual(fork.id, vm.id);
  assert.equal((await fork.info()).forked_from, vm.id);
  await fork.start();
  assert.deepEqual(await fork.files.read('sdk-proof.bin'), data);
  await fork.files.write('sdk-proof.bin', new Uint8Array([42]));
  await vm.start();
  assert.deepEqual(await vm.files.read('sdk-proof.bin'), data);
  await vm.pause();
  assert.equal((await vm.info()).status, VmLifecycleState.SUSPENDED);
  await vm.resume();
  assertOutput((await vm.exec('printf SDK_RESUMED')).stdout, 'SDK_RESUMED');
  assert.deepEqual(await vm.files.read('sdk-proof.bin'), data);
  // On failure the owning service fixture preserves evidence before cleanup.
  await fork.delete();
  await vm.delete();
  assert(!(await hv.list()).sandboxes.some(entry => entry.name === name || entry.name === `${name}-fork`));
  process.stdout.write(`SDK_LIVE_ACCEPTANCE_OK bytes=${String(data.length)} sha256=${digest}\n`);
} finally {hv.close();}
