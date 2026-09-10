# Capsem TypeScript SDK

Async clients for the authenticated HTTP gateway, usable in browsers and Node.
Supply the gateway URL and bearer token explicitly.

```ts
import {Hypervisor, HostLogSource, VM} from '@capsem/sdk';

const hv = new Hypervisor(url, token, {timeoutMs: 120_000});
try {
  const status = await hv.info(); // health, version, profiles and updates
  const vm = await hv.create('code', {name: 'work', vcpu: 4, memory: '8G'});
  const result = await vm.exec('uname -a', {timeout_secs: 60});
  await vm.copy.toVm('/hello.txt', new TextEncoder().encode('hello'));
  const bytes = await vm.copy.fromVm('/hello.txt');
  const info = await vm.info(); // includes AI, network and files
  const stats = await vm.stats.details();
  const logs = await hv.log({source: HostLogSource.GATEWAY, tail: 100});
} finally {
  hv.close();
}

const vm = new VM(url, token, {name: 'work'}); // or {id: canonicalId}
try { const files = await vm.list('/'); }
finally { vm.close(); }
```

Models use runtime enums, exact optional properties, and distinct nullable
values. Runtime validators reject wrong primitive types, unknown enum values,
and integers that JavaScript cannot represent safely. Generated source is
included in strict compilation, lint, coverage, drift and module-size checks.

Named VMs are persistent. Unnamed VMs are ephemeral. Omitted CPU and memory
values use the selected profile's defaults; numeric memory values are MB.
VM names resolve through `hv.list()` and cache the canonical ID. Missing or
ambiguous names fail before a VM operation is sent.

Methods mirror the gateway, including `vm.history()`, `vm.changes(checkpoint)`,
`vm.timeline()`, `vm.snapshots.list/status()` and `vm.stats.summary/details()`.
Query options retain the gateway's spelling, such as `max_bytes` and `trace_id`.
`hv.update()` applies the update. Single-file copy requires a running VM security
ledger; stopped VMs return the gateway's conflict error.

Pass `{signal}` to cancel a call. Choose `timeoutMs` long enough for the command's
`timeout_secs`; the HTTP deadline covers response reading too. `HttpError`
preserves the gateway status and response text. `NetworkError` identifies fetch
or response-body connection failures and retains the original `cause`. Response
validation errors remain distinct; cancellation and timeout reasons are preserved.
Mutations are never retried.

Created/forked handles share their owner's connection. Closing a child leaves
siblings usable; closing the owner invalidates its children. `close()` releases
client access. VM lifecycle operations use `start/stop/pause/resume/delete()`.

Managed restart, snapshot create/restore, mounts and port exposure are pending.

Run `pnpm install --frozen-lockfile`, then `pnpm lint`, `pnpm check`,
`pnpm test`, and `pnpm build` in this directory. The fast gate also builds the
package tarball and verifies generation against `sdk/specification/openapi.json`.

The `@capsem/sdk/operations` and `@capsem/sdk/transport` exports expose generated
endpoint functions for applications that already own their connection flow.
The UI links this package and reads its compiled `dist` exports. Gate commands
and CI install and build the SDK before frontend checks or bundles. When running
frontend commands directly, first install dependencies and run `pnpm build` here.
