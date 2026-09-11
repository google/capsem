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
  cgroups. The container cannot create VSOCK sockets.
- A trusted prestart hook gives the container one veth whose VM end is its only
  gateway. Every port the VM intercepts (`nat OUTPUT` redirects for DNS 53 and the
  HTTP/HTTPS ports) is mirrored as a DNAT to the same loopback proxies, so container
  traffic goes through the host MITM, DNS handler, rules, plugins and ledger exactly
  like VM traffic; anything else arriving from that interface is rejected, so the
  container cannot reach the VM's other listeners or its dummy address. The VM's CA
  bundle is bind-mounted read-only and exported through `SSL_CERT_FILE`,
  `REQUESTS_CA_BUNDLE`, `CURL_CA_BUNDLE` and `NODE_EXTRA_CA_CERTS`; images that pin
  certificates or use private trust stores fail TLS, by design.
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
exit or timeout stops the VM while the CLI is attached, and explicit delete removes it. Existing restart
restores the command and host bindings; fork copies the workload and workspace
without copying host ports. Redis memory and image-declared tmpfs volumes are
fresh on cold boot, so this does not promise database persistence across restart.

## Artifact identities and evidence

Supported ARM64 `code` profile proof: **23 passed in 123.47s** at
`2ade1feeccce7981442093fbcae6f661c3dc9be1`, with no source changes during execution.
`just focus-test kingslanding` completed all 20 steps in 4m17s under the enforced
macOS gate sandbox. The journal is
`cache/target/gate-runs/20260910-135721-6b567b-focus-test/run.jsonl`.
The suite includes eight in-guest adversarial OCI cases and nine existing guest
hardening diagnostics; none of those checks were skipped.

The preceding failures were investigated and fixed:

- `20260910-131254-63da2a`: shared-cache assets replaced the rebuilt OCI kernel.
  S04-004 records the selection regression and full ARM64 profile rebuild
  `20260910-132119-a0c2a3-build-assets` (10 steps, 7m11s).
- `20260910-133003-c0ac3b`: macOS refused nested Seatbelt initialization.
  The gate now hands off only named self-confining children; the router installs
  its stricter profile before accepting data. Native subprocess tests still deny
  files, control sockets, outbound connects, execution and parent signalling;
  ordinary gate children remain unable to reach public networking.
- `20260910-134751-93e25e`: 22 cases passed, but a short shell command lost output.
  The CLI now flushes Tokio stdout/stderr before exiting. Its regression passes
  within the final complete Kingslanding run.

Final asset and fixture SHA-256 identities:

| Artifact | SHA-256 |
|---|---|
| Kernel | `b1d8357da3005ef29c9dd2797e83177b85ad25494b13802a78603b2f0b843892` |
| Initrd | `3f8252afc6482295b7f7fd34b16a57eb7cf35e7ae5dba053a1a798dc92b25464` |
| Rootfs | `205741b3389b118f02c373ea249c4763b71c76d84307a4be49120c0c81a88940` |
| Upstream Redis ARM64 manifest | `f8d15882ba108587477ce13c00ab0551933a84138427b7cc9abadfbe45ffd973` |
| Exported Redis rootfs gzip | `46ad10323cb8da8717d3c72a92f64d8a49ecfd114c824d6f0450791f38b51908` |
| Hermetic registry OCI manifest | `1b770cc2a7abe65129973814d1112d4be5cb05597d9d48484e249ca4fc25a555` |

The gate automatically retained the benchmark under
`cache/target/tests/benchmarks/kingslanding/redis-8tq_d6cl/`: 18 raw trials,
`identity.json` with source/asset/binary identities and executed commands,
`doctor.json` with machine fitness, `benchmarks.db`, and `report.txt`.
Each lane ran three repetitions. Medians in PING/s:

| Clients / pipeline | Host published port | Guest container loopback |
|---|---:|---:|
| 1 / 1 | 9,947 | 35,236 |
| 32 / 1 | 83,741 | 387,435 |
| 32 / 16 | 1,267,753 | 3,933,010 |

These are exploratory measurements, not a sustained-performance guarantee or
release baseline. The fitness check reports pre-existing Capsem processes.
Focused Rust tests and clippy, Ruff, strict gate Ty, formatting, and source guards
passed. The final sandbox/Citadel cohort passed 936 checks with five platform
skips. Earlier native Linux ARM64 router tests and macOS packaging checks remain
separate evidence in Sprinty; Linux package assembly and complete release
qualification are not claimed by this local gate. Historical router line coverage
was 93/136 (68.38%); confined children cannot write profiling files.

## Limits and next work

Next: a minimal `container` profile containing the trusted guest agent, OCI runtime
and required network plumbing, followed by policy-controlled container egress.
Current containers boot from `code` or `co-work`; their application rootfs is already
separate, but the surrounding VM still carries the profile's development tools.

- IPv4 loopback TCP publication only; no UDP, LAN binding, container egress, SDK,
  OpenAPI endpoint or Inspect integration. This does not claim Docker compatibility.
- One container workload per named VM. Client termination detaches. Workload
  completion and timeout stop the VM while the CLI remains attached; after
  detachment its deadline no longer runs, and VM control is explicit. Cold boot reruns
  the saved image command and does not restore process memory or tmpfs contents.
- The host exec transport and CLI combine stdout/stderr. The offline OCI fixture
  tests separate subprocess pipes inside the guest. Automatic image-derived names can race;
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
