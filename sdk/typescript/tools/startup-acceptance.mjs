// A clean installed consumer must outlive both the old wait and its finite lease.
import assert from 'node:assert/strict';
import process from 'node:process';
import {performance} from 'node:perf_hooks';
import {setTimeout as delay} from 'node:timers/promises';
import {uuidV4} from './crypto.mjs';

const packageName = '@capsem/sdk';
/** @type {unknown} */
const built = await import(packageName);
const {Hypervisor, ContainerState, HttpError} = /** @type {typeof import('../src/index.js')} */ (built);
const url = process.env.SDK_GATEWAY_URL, token = process.env.SDK_GATEWAY_TOKEN;
const environmentId = process.env.SDK_ENVIRONMENT_ID, revisionId = process.env.SDK_REVISION_ID;
assert(url && token && environmentId && revisionId);
const environment = {environment_id: environmentId, revision_id: revisionId};
const hv = new Hypervisor(url, token);
try {
  const started = performance.now();
  const scope = await hv.managed({environment, cpus: 4, memory: 12});
  /** @type {string | undefined} */
  let original;
  await scope.run(async vm => {
    assert(performance.now() - started >= 235_000, 'Creation answered before approved startup completed');
    assert.equal(scope.ownership?.lease_seconds, 30);
    const result = await vm.exec('test "$(cat /workspace/long-startup)" = retained; redis-cli PING');
    assert.equal(result.exit_code, 0);
    assert.equal(result.stdout.data, 'PONG\n');
    assert.equal(result.stderr.data, '');
    assert.deepEqual((await vm.info()).environment?.boot, environment);
    assert.equal((await vm.container.status()).state, ContainerState.RUNNING);
    original = vm.id;
  });
  assert(!(await hv.list()).sandboxes.some(row => row.id === original));

  const controller = new AbortController();
  const reason = new Error('Cancel the original preparing owner');
  const tag = uuidV4();
  /** @type {unknown} */
  let failure;
  const opening = hv.withManaged({environment, cpus: 4, memory: 12, labels: {probe: tag}, signal: controller.signal}, () => Promise.reject(new Error('Cancelled startup reached its entrypoint')));
  const outcome = opening.catch(/** @param {unknown} error */ error => {failure = error; return error;});
  /** @type {string | undefined} */
  let preparingId;
  try {
    const deadline = performance.now() + 45_000;
    while (performance.now() < deadline) {
      if (failure !== undefined) throw failure instanceof Error ? failure : new Error('Startup failed', {cause: failure});
      for (const row of (await hv.list()).sandboxes) {
        if (row.labels?.probe !== tag) continue;
        // The VM row can precede its container setup record.
        const status = await hv.vm({id: row.id}).container.status().catch(/** @param {unknown} error */ error => {
          if (error instanceof HttpError && error.status === 404) return undefined;
          throw error instanceof Error ? error : new Error('Container status failed', {cause: error});
        });
        if (status?.state === ContainerState.PREPARING) {
          preparingId = row.id;
          break;
        }
      }
      if (preparingId) break;
      await delay(50);
    }
    assert(preparingId, 'No real preparing workload observed');
    controller.abort(reason);
    assert.equal(await outcome, reason);
    assert(!(await hv.list()).sandboxes.some(row => row.id === preparingId));
  } finally {
    controller.abort(reason);
    await outcome;
  }
  assert.equal((await hv.list()).sandboxes.length, 0);
  process.stdout.write('SDK_STARTUP_DEADLINE_OK lease=renewed preparing=cancelled cleanup=complete\n');
} finally {hv.close();}
