// Exercise inherited startup through a cleanly installed SDK.
import assert from 'node:assert/strict';
import process from 'node:process';
import {performance} from 'node:perf_hooks';
import {uuidV4} from './crypto.mjs';

const packageName = '@capsem/sdk';
/** @type {unknown} */
const installed = await import(packageName);
const {ContainerState, Hypervisor} = /** @type {typeof import('../src/index.js')} */ (installed);
const url = process.env.SDK_GATEWAY_URL, token = process.env.SDK_GATEWAY_TOKEN;
const source = process.env.SDK_COLD_SOURCE_NAME;
const environmentId = process.env.SDK_ENVIRONMENT_ID, revisionId = process.env.SDK_REVISION_ID;
assert(url && token && source && environmentId && revisionId);
const started = performance.now();
const hv = new Hypervisor(url, token);
try {
  const vm = await hv.create({
    from: source,
    name: `cold-package-${uuidV4()}`,
    memory: Number(process.env.SDK_COLD_MEMORY_GIB),
    cpus: Number(process.env.SDK_COLD_CPUS),
  });
  try {
    assert.notEqual(vm.id, process.env.SDK_VM_ID);
    assert(performance.now() - started >= Number(process.env.SDK_COLD_MIN_SECONDS) * 1000);
    const info = await vm.info();
    assert.deepEqual(info.environment?.boot, {environment_id: environmentId, revision_id: revisionId});
    assert.notEqual(info.environment?.owner_generation, process.env.SDK_COLD_SOURCE_GENERATION);
    assert.equal((await vm.container.status()).state, ContainerState.RUNNING);
    const result = await vm.exec('test "$(cat /workspace/entrypoint-count)" = 2 && test "$BOUND_REVISION" = retained-variable && redis-cli PING');
    assert.equal(result.exit_code, 0);
    assert.equal(result.stdout.data, 'PONG\n');
  } finally {
    try {await vm.delete();} finally {vm.close();}
  }
} finally {hv.close();}
process.stdout.write('SDK_COLD_LIFECYCLE_OK\n');
