// Run from a clean installed consumer against an admitted hermetic OCI pin.
import assert from 'node:assert/strict';
import {randomUUID} from 'node:crypto';
import {readFileSync} from 'node:fs';
import process from 'node:process';

const packageName = '@capsem/sdk';
/** @type {unknown} */
const built = await import(packageName);
const {Hypervisor, ContainerState, ExecTarget, VmLifecycleState} = /** @type {typeof import('../src/index.js')} */ (built);
const url = process.env.SDK_GATEWAY_URL, token = process.env.SDK_GATEWAY_TOKEN;
const reference = process.env.SDK_IMAGE, certificate = process.env.SDK_REGISTRY_CA;
assert(url && token && reference && certificate, 'Hermetic gateway/image fixture is required');
const ca = readFileSync(certificate, 'utf8');
const name = `sdk-ts-oci-${randomUUID().slice(0, 8)}`;
const workload = 'printf SDK_OCI_WORKLOAD; test -x /usr/local/bin/redis-cli; test ! -e /var/tmp/capsem-container/workload.pid';
const guest = 'printf SDK_OCI_VM; test -s /var/tmp/capsem-container/workload.pid';

/** @param {import('../src/index.js').VM} vm */
async function entered(vm) {
  // The first operation after create/start/resume must enter the workload.
  const result = await vm.exec(workload);
  assert.equal(result.exit_code, 0);
  assert.equal(result.stdout.data, 'SDK_OCI_WORKLOAD');
  assert.equal(result.stderr.data, '');
  const host = await vm.exec(guest, {target: ExecTarget.VM});
  assert.equal(host.exit_code, 0);
  assert.equal(host.stdout.data, 'SDK_OCI_VM');
  assert.equal(host.stderr.data, '');
}

const hv = new Hypervisor(url, token);
try {
  for (const selectedName of [name, '']) {
    const vm = await hv.create({name: selectedName, cpus: 2, memory: 2, image: reference, registry: {ca_pem: ca}});
    await entered(vm);
    const info = await vm.info();
    assert.equal(info.id, vm.id);
    assert.equal(info.persistent, Boolean(selectedName));
    assert.equal(info.status, VmLifecycleState.RUNNING);
    const status = await vm.container.status();
    assert.equal(status.state, ContainerState.RUNNING);
    assert.equal(status.error ?? null, null);
    assert.equal(status.image, reference);
    assert.equal(status.resolved, reference);
    assert.equal(status.digest, reference.split('@')[1]);
    if (selectedName) {
      const stopped = await vm.stop();
      assert(stopped.success && stopped.persistent);
      assert.equal((await vm.info()).status, VmLifecycleState.STOPPED);
      await vm.start();
      await entered(vm);
      await vm.pause();
      assert.equal((await vm.info()).status, VmLifecycleState.SUSPENDED);
      await vm.resume();
      await entered(vm);
      assert((await vm.delete()).success);
    } else {
      const stopped = await vm.stop();
      assert(stopped.success && !stopped.persistent);
    }
    assert(!(await hv.list()).sandboxes.some(entry => entry.id === vm.id));
  }
  assert.equal((await hv.list()).sandboxes.length, 0);
  process.stdout.write('SDK_OCI_ACCEPTANCE_OK named=1 unnamed=1 workload=entered vm=entered\n');
} finally {hv.close();}
