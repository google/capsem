# Capsem TypeScript SDK

Image catalog and prefetch use the service's admission policy:

```ts
import {Hypervisor} from '@capsem/sdk';

const hv = new Hypervisor(url, token);
try {
  const catalog = await hv.images.list({refresh: false});
  for (const image of catalog.images) console.log(image.name, image.image, image.cached);
  const pulled = await hv.images.pull('code', {timeoutMs: 120_000});
  console.log(pulled.resolved);
} finally { hv.close(); }
```

An absent image pin means no compatible host version; cache `unknown` does
not establish readiness. Pass `registry` to one `pull` when private access is
needed. It is not retained for later calls. Images share the Hypervisor's
connection lifetime and accept per-call cancellation/deadline options.
Prefetch does not create a VM.

`images.list` lists the registry catalog. Its `cached` field reports the
service's local disk observation: `missing`, `partial`, `ready`, or `unknown`
when verification is pending or invalidated. `ready` describes verified local
bytes; image admission and registry freshness remain separate decisions.

Async clients for the authenticated HTTP gateway, usable in browsers and Node.
Supply the gateway URL and bearer token explicitly.

```ts
import {Hypervisor, HostLogSource, VM} from '@capsem/sdk';

const hv = new Hypervisor(url, token, {timeoutMs: 120_000});
try {
  const status = await hv.info(); // health, version, assets and updates
  const network = await hv.networks.create('private');
  const vm = await hv.create({
    name: 'work', cpus: 4, memory: 8, networks: [network],
    image: 'docker.io/library/nginx:alpine', command: ['nginx', '-g', 'daemon off;'],
    env: {MODE: 'preview'},
  });
  const port = await vm.ports.open(80, {authenticate: true});
  console.log(port.url);
  await hv.networks.logs(network, {vm: vm.id});
  const result = await vm.exec('uname -a', {timeout_secs: 60});
  console.log(result);
  await vm.files.write('/workspace/hello.txt', new TextEncoder().encode('hello'));
  const bytes = await vm.files.read('/workspace/hello.txt');
  const info = await vm.info(); // includes AI, network and files
  const stats = await vm.stats.details();
  await vm.persist('saved-workspace');
  const triage = await hv.debug.triage({vm_id: vm.id, since: '1h'});
  const server = await hv.mcp.get('filesystem');
  const tools = await server.tools.list();
  const logs = await hv.log({source: HostLogSource.GATEWAY, tail: 100});
  await vm.ports.close(port);
} finally {
  hv.close();
}

const vm = new VM(url, token, {name: 'work'}); // or {id: canonicalId}
try { const files = await vm.files.list(); }
finally { vm.close(); }
```

`vm.exec(command, {target: ExecTarget.VM})` explicitly selects VM diagnostics;
`target: ExecTarget.WORKLOAD` selects the OCI workload. Import `ExecTarget`
from `@capsem/sdk`. Omitted or `null` target keeps the service default: workload
when present, otherwise VM. A refused workload target is returned as an error;
the SDK does not switch targets or replay the command.

Models use runtime enums, exact optional properties, and distinct nullable
values. Runtime validators reject wrong primitive types, unknown enum values,
and integers that JavaScript cannot represent safely. Generated source is
included in strict compilation, lint, coverage, drift and module-size checks.

Named VMs are persistent. Unnamed VMs are ephemeral. Omitted CPU and memory
values use the service defaults (4 CPUs, 12 GiB); memory is a positive integer
in GiB.
VM names resolve through `hv.list()` and cache the canonical ID. Missing or
ambiguous names fail before a VM operation is sent.

Methods mirror the gateway, including `vm.history()`, `vm.timeline()` and
`vm.stats.summary/details()`.
Query options retain the gateway's spelling, such as `max_bytes` and `trace_id`.
`hv.update()` applies the update. File access requires a running VM security
ledger; stopped VMs return the gateway's conflict error.

Pass `{signal}` to cancel a call. Choose `timeoutMs` long enough for the command's
`timeout_secs`; the HTTP deadline covers response reading too. `HttpError`
preserves the gateway status and response text. `NetworkError` identifies fetch
or response-body connection failures and retains the original `cause`. Response
validation errors remain distinct; cancellation and timeout reasons are preserved.
Mutations are never retried.
The guest exec channel preserves separate `stdout` and `stderr` lanes. Each is
a typed `ExecOutput`; `decodeExecOutput()` returns its exact
bytes regardless of whether the wire value uses UTF-8 or base64.

Created/forked handles share their owner's connection. Closing a child leaves
siblings usable; closing the owner invalidates its children. `close()` releases
client access. VM lifecycle operations use `start/stop/pause/resume/delete()`.

`await hv.restart()` returns a typed HTTP 202 acknowledgement. It requires an
idle service managed by launchd or systemd; active/starting VMs return 409 and
an unmanaged service returns 503. The gateway rotates its token on restart.
Obtain fresh credentials and construct a new client explicitly; never replay
the restart call. Acceptance does not claim reconnection has completed.

`hv.networks` provides typed create/list/inspect/delete and cursor-based audit
logs; pass returned network objects directly to creation. VM membership uses
`vm.networks.list/attach/detach`, with typed network objects.
`image` selects a container workload, with `command`, `env`, and a typed
`Registry` as optional settings. Creation returns after HTTP reports the workload
ready, while `vm.container.status()` remains a read-only diagnostic.
`vm.ports.open` creates a plain loopback listener by default and uses the
browser-authentication flow when `authenticate` is true. The SDK infers whether
the workload target is the container or VM. `list` and `close` manage the same
typed port objects.
Mounts remain pending.

`hv.run(command)` executes once in a temporary VM. `hv.debug.panics()`, `hv.debug.triage()`
and `hv.purge()` expose diagnostics and cleanup. `hv.mcp` covers the MCP
servers every VM runs (settings.toml `[mcp]` with corp's laid over it):
`info()`, `servers()`, `defaultPermission()` and `get(name)`, whose server has
`tools.list()`, `tools.call(name, args)` and `refresh()`; tool arguments and
results retain their native JSON shape.

Run `pnpm install --frozen-lockfile`, then `pnpm lint`, `pnpm check`,
`pnpm test`, and `pnpm build` in this directory. The fast gate also builds the
package tarball and verifies generation against `sdk/specification/openapi.json`.
Before running package acceptance directly, run `pnpm prewarm:package` once
to cache runtime dependencies and the compiler used by its contamination test.
`pnpm exec vitest run tests/package-install.test.ts` packs the current build,
installs it offline into a temporary consumer outside the checkout, verifies
payload hashes and export origins, and exercises authenticated image requests.
It rejects modified payloads, linked installs and development dependencies,
then removes the consumer. The gate declares prewarming outside its sandbox.

The `@capsem/sdk/operations` and `@capsem/sdk/transport` exports expose generated
endpoint functions for applications that already own their connection flow.
The UI links this package and reads its compiled `dist` exports. Gate commands
and CI install and build the SDK before frontend checks or bundles. When running
frontend commands directly, first install dependencies and run `pnpm build` here.
