import assert from 'node:assert/strict';
import process from 'node:process';

// Load the built package exports; source types also check this tool on a cold checkout.
const packageName = '@capsem/sdk';
/** @type {unknown} */
const built = await import(packageName);
const sdk = /** @type {typeof import('../src/index.js')} */ (built);
const {Hypervisor, VM, HttpError, FileChangeKind} = sdk;
const url = process.env.SDK_GATEWAY_URL, token = process.env.SDK_GATEWAY_TOKEN, id = process.env.SDK_VM_ID;
assert(url && token && id, 'SDK acceptance fixture must provide gateway credentials and VM ID');

const denied = new Hypervisor(url, 'incorrect-token');
try {await assert.rejects(denied.list(), error => error instanceof HttpError && error.status === 401);}
finally {denied.close();}

const hv = new Hypervisor(url, token);
const vm = new VM(url, token, {name: 'route-workspace'});
try {
  assert((await hv.info()).gateway_version.length > 0);
  const profiles = await hv.profiles.list();
  const [profile] = profiles;
  assert(profile, 'SDK acceptance fixture must expose at least one profile');
  assert.equal((await hv.profiles.mcp(profile.id).info()).profile_id, profile.id);
  assert(Array.isArray((await hv.debug.panics({limit: 2})).panics));
  assert.equal(typeof (await hv.debug.triage({since: '1h', limit: 2})).session, 'object');
  assert((await hv.list()).sandboxes.some(entry => entry.id === id));
  const files = await vm.files.list('/');
  assert.equal(vm.id, id);
  assert(files.entries.some(entry => entry.name === 'created.txt'));
  const snapshots = await vm.snapshots.list();
  assert.equal(snapshots.total, 1);
  assert.equal(snapshots.snapshots[0]?.checkpoint, 'cp-10');
  const changes = await vm.files.history('cp-10');
  assert.deepEqual(new Map(changes.changes.map(entry => [entry.path, entry.kind])), new Map([
    ['created.txt', FileChangeKind.CREATED], ['modified.txt', FileChangeKind.MODIFIED],
    ['deleted.txt', FileChangeKind.DELETED],
  ]));
  for (const call of [() => vm.files.read('/created.txt'), () => vm.files.write('/refused.txt', new Uint8Array([1]))]) {
    await assert.rejects(call, error => error instanceof HttpError && error.status === 409
      && error.body.includes('running sandbox security ledger'));
  }
  assert(!(await vm.files.list()).entries.some(entry => entry.name === 'refused.txt'));
} finally {vm.close(); hv.close();}

const direct = new VM(url, token, {id});
try {assert.equal((await direct.snapshots.status()).total, 1);}
finally {direct.close();}
process.stdout.write('BRAAVOS_SDK_ACCEPTANCE_OK\n');
