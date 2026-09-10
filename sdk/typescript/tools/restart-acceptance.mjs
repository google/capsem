import assert from 'node:assert/strict';
import process from 'node:process';
const packageName = '@capsem/sdk';
/** @type {unknown} */
const built = await import(packageName);
const {Hypervisor, RestartAuthentication, RestartStatus, ServiceManager} =
  /** @type {typeof import('../src/index.js')} */ (built);

const url = process.env.SDK_GATEWAY_URL, token = process.env.SDK_GATEWAY_TOKEN;
assert(url && token, 'SDK restart fixture must provide gateway credentials');
const hv = new Hypervisor(url, token);
try {
  const response = await hv.restart();
  assert.equal(response.status, RestartStatus.ACCEPTED);
  assert.equal(response.authentication, RestartAuthentication.NEW_TOKEN_REQUIRED);
  assert.equal(response.manager, ServiceManager.LAUNCHD);
} finally {
  hv.close();
}
process.stdout.write('SDK_RESTART_ACCEPTED\n');
