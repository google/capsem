import assert from 'node:assert/strict';
import process from 'node:process';

const packageName = '@capsem/sdk';
/** @type {unknown} */
const built = await import(packageName);
const {Hypervisor} = /** @type {typeof import('../src/index.js')} */ (built);
const url = process.env.SDK_GATEWAY_URL, token = process.env.SDK_GATEWAY_TOKEN;
assert(url && token, 'Fixture gateway URL and token are required');
const hv = new Hypervisor(url, token, {timeoutMs: 120_000});
try {
  const scope = await hv.managed({cpus: 4, memory: 12});
  const sessionId = scope.vm.id;
  assert.equal(scope.ownership?.lease_seconds, 300);
  assert.equal(scope.ownership.renew_after_seconds, 60);
  await scope.run(async vm => {
    const info = await vm.info();
    assert.equal(info.id, sessionId);
    assert.equal(info.persistent, false);
    assert.equal(info.cpus, 4);
    assert.equal(info.ram_mb, 12288);
    const result = await vm.exec('printf MANAGED_SDK_NATIVE_READY', {timeout_secs: 60});
    assert.equal(result.exit_code, 0);
    assert.equal(result.stdout.encoding, 'utf8');
    assert.equal(result.stdout.data, 'MANAGED_SDK_NATIVE_READY');
  });
  assert(!(await hv.list()).sandboxes.some(row => row.id === sessionId));
  await scope.close();
  assert(!(await hv.list()).sandboxes.some(row => row.id === sessionId));
  process.stdout.write('SDK_MANAGED_ACCEPTANCE_OK\n');
} finally {hv.close();}
