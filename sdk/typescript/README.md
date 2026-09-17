# Capsem TypeScript SDK

Async clients for the authenticated HTTP gateway, usable in browsers and Node.
Supply the gateway URL and bearer token explicitly.

```ts
import {Hypervisor, HostLogSource, VM} from '@capsem/sdk';

const hv = new Hypervisor(url, token, {timeoutMs: 120_000});
try {
  const status = await hv.info(); // health, version, profiles and updates
  const [profile] = await hv.profiles.list();
  if (profile === undefined) throw new Error('No Capsem profile is available');
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
  await vm.files.write('/hello.txt', new TextEncoder().encode('hello'));
  const bytes = await vm.files.read('/hello.txt');
  const info = await vm.info(); // includes AI, network and files
  const stats = await vm.stats.details();
  await vm.persist('saved-workspace');
  const triage = await hv.debug.triage({vm_id: vm.id, since: '1h'});
  const server = await hv.profiles.mcp(profile).get('filesystem');
  const tools = await server.tools.list();
  const logs = await hv.log({source: HostLogSource.GATEWAY, tail: 100});
  await vm.ports.close(port);
} finally {
  hv.close();
}

const vm = new VM(url, token, {name: 'work'}); // or {id: canonicalId}
try { const files = await vm.files.list('/'); }
finally { vm.close(); }
```

Models use runtime enums, exact optional properties, and distinct nullable
values. Runtime validators reject wrong primitive types, unknown enum values,
and integers that JavaScript cannot represent safely. Generated source is
included in strict compilation, lint, coverage, drift and module-size checks.

Named VMs are persistent. Unnamed VMs are ephemeral. Omitted CPU and memory
values use the selected profile's defaults; memory is a positive integer in GiB.
Omit `profile` for the profile the gateway's catalog names as its default,
which the client reads once from `GET /status` and caches. To select another
profile, pass an object from `await hv.profiles.list()` to `create` or `run`.
VM names resolve through `hv.list()` and cache the canonical ID. Missing or
ambiguous names fail before a VM operation is sent.

Methods mirror the gateway, including `vm.history()`, `vm.files.history(checkpoint)`,
`vm.timeline()`, `vm.snapshots.list/status()` and `vm.stats.summary/details()`.
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
Snapshot create/restore and mounts remain pending.

`hv.run(command)` executes once in a temporary VM. `hv.debug.panics()`, `hv.debug.triage()`
and `hv.purge()` expose diagnostics and cleanup. `hv.profiles` provides typed
profile and MCP discovery, refresh, permissions and tool calls; tool arguments
and results retain their native JSON shape.

Run `pnpm install --frozen-lockfile`, then `pnpm lint`, `pnpm check`,
`pnpm test`, and `pnpm build` in this directory. The fast gate also builds the
package tarball and verifies generation against `sdk/specification/openapi.json`.

The `@capsem/sdk/operations` and `@capsem/sdk/transport` exports expose generated
endpoint functions for applications that already own their connection flow.
The UI links this package and reads its compiled `dist` exports. Gate commands
and CI install and build the SDK before frontend checks or bundles. When running
frontend commands directly, first install dependencies and run `pnpm build` here.
