# OCI containers in Capsem

This worktree implements `capsem run docker://IMAGE` and loopback TCP publishing:

```sh
capsem run -p 6379:6379 docker://redis:7.4.11-alpine
```

The worktree build pulls the native Linux image, creates an image-named VM,
uses the image's entrypoint and default command, streams logs, and retains a
named VM controlled by the existing lifecycle commands. A command after
the image overrides its default command. Registry-qualified references also
work; `docker://` disambiguates short images from the existing shell-run command.
Installed binaries, services, and profiles are not changed by this spike.

## Ownership and confinement

- Host image pulling uses `oci-client` with the existing WebPKI trust stack.
  Explicit registry CA trust and username/token authentication stay on the host.
  Authentication happens before cache access. Cached blobs are origin/auth scoped,
  digest checked on reuse, atomically published, and owned by the existing cache
  contract. Concurrent pulls share verified blobs; cancellation cannot publish
  partial data. There is no Docker daemon or registry client inside the VM.
- Packaged `umoci` unpacks layers inside the guest. The trusted launcher replaces
  image hooks and runtime configuration, creates a read-only root, bounded writable
  scratch/volumes, restricted capabilities, private namespaces and CPU/memory/pids
  cgroups. The container cannot create VSOCK sockets. A trusted prestart hook brings
  up only its private loopback interface before the image entrypoint executes.
- `-p HOST:GUEST` binds IPv4 loopback on the host. Host port zero chooses an unused
  port and prints it. Each mapping owns a Rust companion with a two-worker async
  runtime, at most 128 active connections, bounded queues and setup deadlines.
  A VM permits at most eight mappings; the guest bridge permits 128 connections.
- The VM process binds the listener and grants connected data descriptors. The
  companion carries TCP bytes without running them through the VM control API.
  Its only requests are fixed nine-byte connection/close records. It cannot select
  another guest port or ask the parent to open arbitrary files or sockets.
- macOS Seatbelt and Linux seccomp deny new files, outbound connections, process
  execution and signalling other processes. Inherited descriptors are closed at
  process entry. The router has no virtualization entitlement. Tests attempt to
  read hypervisor state and connect to a control socket after confinement.
- VSOCK service 5008 carries data. A disposable guest setup thread enters the
  running container's network namespace, connects to its loopback TCP port, then
  hands the sockets to two reusable async workers. Runtime workers never call
  `setns`. Half-close, binary payloads, refused connections, parent death and
  listener reuse are exercised.

The router uses foundation-owned descriptor channels to work around two observed
`tokio-unix-ipc` 0.4 ownership bugs: serialized owned handles leak sender FDs, and
closing before Tokio deregistration leaves stale Linux epoll registrations for
duplicated sockets. Borrowed handle grants and deregistration before close fix
these without widening the sandbox. Separate owned reader tasks avoid cancelling
a partial frame read when another async branch becomes ready.

## Kingslanding

The container E2E suite lives in `tests/ironbank/kingslanding/`, using the existing
Ironbank service and VM helpers. Run it through its owner:

```sh
python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 1800 -- just focus-test kingslanding
```

After changing kernel or profile package inputs, first rebuild that profile with
`just build-assets arm64 code` (use `x86_64` on that host). The focused gate
uses the invoking checkout's assembled assets, refreshes its guest/host binaries,
prepares a digest-pinned native Redis image,
and runs the suite with public networking blocked. The fixture supports ARM64 and
x86_64; cache reuse requires matching image identity and a verified archive hash.
Preparation uses Docker only on the development host to export a never-started
image. No Docker daemon runs inside Capsem. The regular functional gate runs
Kingslanding for each profile; benchmark repetitions are omitted from release
rehearsal to avoid recording the same cohort twice.

The suite covers CLI defaults, image cache reuse, logs, exit/timeout behavior,
client detachment, stop/restart/fork/delete, OCI layer semantics, resource limits,
filesystem/network isolation, router confinement and concurrent Redis traffic.
All services and VMs belong to test fixtures; installed services and profiles are
untouched. Throughput results are exploratory measurements, not a release threshold.

A named VM retains its image and command. Closing the client detaches. Workload
exit or timeout stops the VM, and explicit delete removes it. Existing restart
restores the command and host bindings; fork copies the workload and workspace
without copying host ports. Redis memory and image-declared tmpfs volumes are
fresh on cold boot, so this does not promise database persistence across restart.

## Artifact identities and evidence

Kingslanding native ARM64 run: **23 passed in 128.95s**, using the rebuilt
`container-lifecycle-binaries` cohort. Evidence is under
`cache/target/tests/kingslanding-native/`. The lifecycle RED cases are recorded in
Sprinty S04-001; suite ownership and cache RED cases are in S04-003.
The first sandboxed gate run exposed a stale shared-cache kernel; S04-004
records the selection fix and the complete ARM64 profile rebuild
`20260910-132119-a0c2a3-build-assets` (10 steps, 7m11s).

The measurements below predate the lifecycle changes and are retained as
historical spike evidence.

Earlier ARM64 rebuild: `20260910-070738-84e38e-pack-initrd`, seven steps passed in
1m38s. The manifest step rebuilt the changed native dependency graph, explaining
its increase from the earlier 14.8s median to 27.4s.

| Artifact | SHA-256 |
|---|---|
| Kernel | `b1d8357da3005ef29c9dd2797e83177b85ad25494b13802a78603b2f0b843892` |
| Initrd | `9ab41d785d688510c2d7f92470637027ee73328e2ca950116037b2c9b128ffc9` |
| Rootfs | `a41d85bb3cfdc624f47be2e80706d7b2d16e5417beebc8546b8514f377cc388e` |
| Upstream Redis 7.4.11-alpine ARM64 manifest | `f8d15882ba108587477ce13c00ab0551933a84138427b7cc9abadfbe45ffd973` |
| Exported Redis rootfs gzip | `9fc9018f96d3e34e341f813d7a2eafc85d0801b25d13328d198ad1f0a166ad71` |
| Hermetic registry OCI manifest | `560c849ae08f0976d13d9424dcd3db8cc2755c0b19e4536d1fb177edd1a2a3bf` |

`cache/target/tests/container-final` contains the final real-VM test run and
benchmark artifacts: 19 tests passed in 145.81 seconds against the rebuilt cohort. Earlier RED/green evidence remains in the `container-run-*`
and `container-publish-*` directories and Sprinty items S02-002/008/009/010 and
S03-001/002/003. Source-only guards, native tests and package tests are separate
from actual VM execution. macOS package assembly/signing tests run locally;
Linux router execution is in a sealed native ARM64 Linux container. Router line
coverage is 93/136 (68.38%); confined children intentionally cannot write profiling
files. The final source-guard/coverage/package run passed 928 checks in 35.16s;
15 Linux packaging checks were skipped on macOS. An earlier path-ownership
failure was fixed before that rerun. Cargo clippy passed
for the eight affected runtime crates. Linux package assembly is not proven on
this macOS host. Two failed Linux diagnostic containers outlived their client
timeouts; they were identified by their worktree mounts and test commands and
explicitly removed during final cleanup. Successful Linux tests left no container.

The offline suite proves separate stdout/stderr, exit status, all byte values,
read-only root, inaccessible unmounted guest files, isolated networking, denied
VSOCK, memory OOM, process EAGAIN and CPU throttling. Timeout/cancellation check
descendants, runtime directories, mounts and cgroups; fresh VMs cannot share files.
The CLI suite adds default Redis startup, live output above 10 MiB, shell-run
compatibility, image failures, scoped TLS trust and client detachment.
Publication tests add 64 clients using 32 workers, binary Redis SET/GET, guest-root
versus container namespace isolation, port collisions, router crash, VM-owner death
and teardown with slow consumers.

Benchmark output is owned by `capsem-bench-rs`, on both host and guest. Each lane
runs three repetitions: C1/P1 with 10,000 PINGs, C32/P1 with 100,000 and C32/P16
with 800,000. Raw JSON, commands, binary/asset hashes, machine fitness, SQLite
statistics and a report are retained. Latency is measured per pipeline batch.
The final run produced host medians of 9,743, 87,443 and 1,279,009 PING/s
respectively; guest-local medians were 35,567, 396,397 and 3,952,411 PING/s. This is exploratory throughput, not a release baseline:
`doctor` flags already-running Capsem processes, and the short trials do not
establish sustained performance or a regression threshold.

## Limits and next work

Next: a minimal `container` profile containing the trusted guest agent, OCI runtime
and required network plumbing, followed by policy-controlled container egress.
Current containers boot from `code` or `co-work`; their application rootfs is already
separate, but the surrounding VM still carries the profile's development tools.

- IPv4 loopback TCP publication only; no UDP, LAN binding, container egress, SDK,
  OpenAPI endpoint or Inspect integration. This does not claim Docker compatibility.
- One container workload per named VM. Client termination detaches. Workload
  completion and timeout stop the VM; deletion remains explicit. Cold boot reruns
  the saved image command and does not restore process memory or tmpfs contents.
- CLI logs currently merge stdout/stderr. The existing captured exec API and offline
  fixture still prove them separately. Automatic image-derived names can race;
  the service rejects a collision instead of silently attaching to another VM.
- Read-only images needing additional writable paths can fail. Declared writable
  volumes are bounded ephemeral tmpfs, with no persistent volume or mount API.
- `runc --rootless=true` selects its cgroup manager while runc runs as guest root;
  this is not proof of user namespaces or rootless container execution. BPF stays
  disabled; device confinement relies on private devices and restricted capability
  policy. Redis starts through its own entrypoint and user handling.
- Actual VM evidence is ARM64 Apple VZ. The earlier x86_64 kernel build passed;
  no x86_64 container execution or Linux KVM VM proof is claimed here.
- Existing unrelated cache-producer debt remains tracked as S02-007; the legacy
  main-checkout release scratch paths prevent claiming a globally clean cache audit.
