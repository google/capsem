# Inspect AI / container feasibility spike

Status: offline OCI execution proved on rebuilt ARM64 assets in real Apple VZ
VMs. x86_64 kernel compilation passed; no x86_64 runtime proof.

Worktree: `/Users/elie/git/capsem-ai-eval-spike`
Branch: `worktree/ai-eval-containers`
Source baseline: `a6dc86a43`
Sprinty: worktree-local `.sprinty`, baseline `S01-001`, implementation `S02-001`.

## Real Redis image proof

The subsequent Redis sprint proves the upstream `redis:7.4.11-alpine` ARM64 image
in two real Capsem VMs. It reuses the rebuilt ARM64 assets above; no additional
kernel, profile, service or installed-package changes were needed. This remains
an explicitly invoked spike, not yet `capsem run IMAGE` product support.

`tests/fixtures/oci/redis-image.json` pins the platform manifest to
`sha256:f8d15882ba108587477ce13c00ab0551933a84138427b7cc9abadfbe45ffd973`.
Host preparation pulls that digest and exports a never-started Docker container,
removing its temporary container and volume afterward. Redis only executes inside
Capsem. The offline test stages 1 MiB chunks through the existing file API and
checks the archive hash in the guest before extracting its trusted rootfs.
The runtime uses the image's real `redis-server` and `redis-cli`, with explicit
test arguments instead of its Docker entrypoint script.

Passing behavior:

- `runc exec` runs the image's client: PING/PONG and SET/GET return exact values.
- SAVE writes an RDB to writable scratch; a graceful TERM and restart reload it.
- Redis's 8 MiB no-eviction limit rejects writes while PING and GET still work.
- With Redis's own limit removed, a bounded 128 MiB allocation hits the container's
  64 MiB limit. Redis exits 137 and the kernel names its cgroup in an OOM-kill event.
- Forced KILL also exits 137. No runtime entries, cgroups or mounts remain after
  teardown; the temporary bundle directory is removed.
- Redis TCP is confined to its own loopback. Exec sees Alpine, UID 65534 and the
  server's network namespace; it cannot see the initramfs or guest agent.
- A fresh VM starts with an empty database and cannot see the first VM's saved
  RDB. The first VM retains that RDB while the second completes independently.
- The surrounding VM's mount namespace and initramfs remain intact. The execution
  ledger's result and byte counts match after the database shutdown barrier.

The final Redis proof plus the original OCI suite passed together: **4 tests in
12.25s**, including the original eight adversarial guest cases.

### Redis reproduction and identities

After the original ARM64 build/materialization prerequisites, run:

```sh
python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 240 -- \
  uv run --project build_system --frozen python tests/fixtures/oci/prepare_redis.py
python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 240 -- \
  env CAPSEM_RELEASE_BIN_DIR=/Users/elie/git/capsem/cache/target/cargo/release \
  uv run --project build_system --frozen pytest -c build_system/pyproject.toml \
  --rootdir . tests/ironbank/redis_acceptance.py tests/ironbank/test_oci_container.py \
  -q --basetemp=cache/target/tests/redis-final
```

Only preparation requires the registry. Test execution uses local files and real
isolated `ServiceInstance` VMs. `redis_acceptance.py` deliberately requires an
explicit pytest path; normal test discovery has no new public-network dependency.
Generated archives and evidence remain under ignored `cache/target/tests/`.

| Tested artifact | SHA256 |
|---|---|
| Exported compressed rootfs | `9fc9018f96d3e34e341f813d7a2eafc85d0801b25d13328d198ad1f0a166ad71` |
| Image's Redis server executable | `6f95a4ecc8f2da0fea1a4c705ead71d3004e7542c38b985950bf848463da9bff` |

`redis-image/redis-image.json` records the prepared archive identity. Each run's
`redis-evidence-0.json` and `redis-evidence-1.json` also capture RDB identities,
memory denial and the kernel OOM event. Export metadata may vary across Docker
versions; the upstream platform manifest stays pinned and every archive is hashed.

### Findings that the synthetic workload missed

The first upload exceeded the file API body limit; chunking solves staging for
this test without changing that API. More significantly, Capsem boots its agents
inside `/newroot` using chroot. A bare runc exec joined the container's mount
namespace but reached its old initramfs root, instead of the container root.
The fixture now first enters PID 1's root, creates a private mount namespace,
makes its mounts private, and moves `/newroot` onto `/`. Runc then creates its
container underneath this correctly rooted namespace. This only changes the
fixture's mount namespace. A production container launcher must own this setup;
bare runc exec from the existing guest shell is not established as safe.

The initial memory assertion also expected a single oversized Redis command to
be rejected. Redis checks maxmemory before commands, so one command can overshoot;
the corrected test uses a bounded sequence, then independently proves the kernel
limit. See [Redis memory-limit semantics](https://redis.io/docs/latest/develop/reference/eviction/).
Graceful shutdown uses TERM because PID 1 exiting can kill redis-cli SHUTDOWN
before the client exits. Redis warns about the guest's existing overcommit setting;
this proof covers synchronous SAVE, not background persistence or replication.

The user's next requested product surface is Docker-style `capsem run IMAGE`:
resolve an image, run its default command attached to the terminal, and destroy
the disposable VM afterward. That requires separate image-staging and lifecycle
implementation; this Redis proof does not claim those are already available.

## Delivered offline proof

The existing profile builder packages Debian `runc` in both profile package
lists. Both kernel inputs enable mount/PID/UTS/IPC/network namespaces and cgroup
CPU, memory and process limits. Guest initialization mounts cgroup v2 with
`nosuid,nodev,noexec`, failing boot explicitly if that mount fails. The production
and configuration change is 40 lines including the changelog; no service, SDK,
router, public API, registry client or Docker daemon was added.

Ironbank uploads a local fixture through the existing file API and executes it
through the existing VM execution API. The fixture assembles a native OCI rootfs
from the profile's Python executable, standard library and shared libraries. It
records file modes and SHA256 digests, plus each generated OCI config digest.
Container state lives under guest-local `/var/tmp`, with an explicit writable
scratch bind mount and a read-only root. A disposable service owns the VMs and
their teardown; the installed service and installed profiles were not changed.

Workloads run as UID/GID 65534, with empty capabilities, no-new-privileges,
private namespaces and private devices. Scratch is `nodev,nosuid,noexec`.
AF_VSOCK socket creation is denied by seccomp; IPv4 has no route in its separate
network namespace. No container traffic is connected to the future router.

**Runtime limitation:** the fixture invokes `runc --rootless=true` to select its
rootless cgroup manager while launching runc as guest root. This is not proof of
rootless process execution or user namespaces. The packaged runtime's normal
cgroup manager requires device-filter BPF; `CONFIG_BPF_SYSCALL=n` stays intact.
This mode tolerates unavailable device filtering but still enforces the requested
CPU/memory/pids controllers. Device confinement here relies on non-root execution,
empty capabilities, private `/dev` and `nodev` scratch, all exercised by the tests.
It is a fixed trusted test bundle, not a policy for accepting arbitrary OCI configs.

### Acceptance evidence

Executed 2026-09-10 UTC in this worktree:

| Proof | Result |
|---|---|
| Rebuilt ARM64 code profile, kernel and initrd | Passed, 16m33s |
| New Ironbank suite | 3 passed in 6.90s; includes 8 guest adversarial cases |
| Existing full doctor ledger suite | 2 passed in 47.82s, including guest protocol/security checks |
| Existing kernel/init builder tests | 33 passed, 169 unrelated tests deselected |
| Citadel shape/lint coverage and immutable-binary checks | 67 passed, no skips |
| Ruff, ty, shell syntax, Markdown and diff whitespace | Passed |
| x86_64 kernel-only build | Passed, 5m39s; no x86_64 VM execution |

The eight guest cases prove separate stdout/stderr, exit 23, PID 1 and non-root
identity, all 256 byte values round-tripped through the container and host file
API, hidden unmounted guest files, EROFS on root modification, denied device
creation, a distinct network namespace, IPv4 ENETUNREACH and VSOCK EPERM.
They also prove BPF syscall ENOSYS, a 64 MiB memory limit with observed OOM kill,
a 16-process limit with fork EAGAIN, and CPU quota 10000/100000 with observed
throttling. Deadline and forced cancellation exercise live descendants; each
teardown verifies no runtime entries, container cgroup or container mounts remain.
A missing rootfs fails launch cleanly. A separate test proves two fresh VMs do
not share guest-local files. Execution ledger fields and byte counts are checked
after the database-owned shutdown barrier.

Timeout/cancellation here prove the fixture's bounded runc lifecycle. They do not
establish future SDK cancellation semantics or recovery from an arbitrarily
killed host client. The outer disposable VM remains the isolation boundary.

### Reproduction

From the spike worktree, using the normal supported build rails:

```sh
uv sync --project build_system --frozen
python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 3600 -- just _build-assets code arm64
python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 120 -- just _materialize-config
python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 1200 -- just _build-kernel x86_64 code
```

For this run, unchanged host executables were reused from the original checkout
through the harness's supported `CAPSEM_RELEASE_BIN_DIR` override. The doctor
harness requires `cache/target/cargo/debug/capsem-mock-server` in the worktree;
the existing same-source debug binary was copied there. On another machine,
build the host executables and mock server with the existing build rails first.
The profiles and VM assets are resolved from this worktree, not that binary path.

```sh
mkdir -p cache/target/tests
python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 180 -- \
  env CAPSEM_RELEASE_BIN_DIR=/Users/elie/git/capsem/cache/target/cargo/release \
  uv run --project build_system --frozen pytest -c build_system/pyproject.toml \
  --rootdir . tests/ironbank/test_oci_container.py -q --basetemp=cache/target/tests/oci-final
python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 600 -- \
  env CAPSEM_RELEASE_BIN_DIR=/Users/elie/git/capsem/cache/target/cargo/release \
  uv run --project build_system --frozen pytest -c build_system/pyproject.toml \
  --rootdir . tests/ironbank/test_doctor_ledger.py -q --basetemp=cache/target/tests/oci-doctor
python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 120 -- \
  uv run --project build_system --frozen pytest build_system/tests/image/test_docker.py -q -k 'kernel or init'
python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 120 -- \
  uv run --project build_system --frozen pytest -c build_system/pyproject.toml --rootdir . \
  tests/citadel/test_shape_boundaries.py tests/citadel/test_lint_coverage.py \
  tests/capsem-security/test_binary_perms.py -q -rs
```

### Artifact identities and failure history

ARM64 uname: `6.18.44 aarch64`. Runtime package:
`runc 1.1.5+ds1-1+deb12u1`, from the configured Debian snapshot. These identities
describe the tested artifacts, not a recommendation for arbitrary image execution.

| Artifact | SHA256 |
|---|---|
| ARM64 kernel | `b1d8357da3005ef29c9dd2797e83177b85ad25494b13802a78603b2f0b843892` |
| ARM64 initrd | `47336f5f1b215690da5e131af2988d1f47c7f1e0c09e23e005209ba59f463e76` |
| ARM64 profile rootfs | `d91431b06fb14b0150de87bb29cb50ee6aad3bbf746be08b392a8df4929e52a7` |
| ARM64 runc executable | `69c840eb82a8ee095b357edeee8bb86ebca2c899d5d054b2f280bc9d8f9b5d29` |
| OCI rootfs file manifest | `cf74447cab3ffdd5f50f8f364aefbe4127470222ce0c99fa06c9ff8f00bf60c9` |
| x86_64 kernel | `3cc14c7f501d4a2891468e3ba50ffa325c57a6dbd7770443e47ca15bfea58f11` |

Evidence files live in `cache/target/tests/oci-final`: `oci-platform.json`,
`oci-result.json` and `oci-rootfs-manifest.json` in their pytest case directories.
Asset identities are also recorded in `cache/target/assets/manifest.json` and
`B3SUMS`. Gate runs are `20260910-022639-946938-build-assets` (ARM64) and
`20260910-024845-79e0e3-build-assets` (x86_64); the current digest records both
builds green. The ARM64 journal observed the co-work package-list edit during
the code-profile build, so it is diagnostic build evidence, not unchanged-tree
release qualification. The final code-profile inputs match those tested.
The co-work package selection was aligned but its rootfs was not separately built.

The first behavioral test failed on the old assets with `runc: not found`.
The first rebuilt-container run failed on unavailable device BPF; diagnosis led
to the explicit cgroup-manager choice above, preserving kernel hardening. That
run also exposed runc's JSON `null` for an empty runtime list, now accepted along
with `[]`. An initial doctor invocation failed on the missing worktree mock
binary, then passed with that prerequisite supplied. No full release gate,
Docker compatibility or Inspect integration is claimed.

## Goal and integration direction

Run AI evaluations in Capsem, targeting Inspect AI. The user reports that an
integration SDK is forthcoming and supplied the proposed interface below. It is
being built elsewhere; implementation and wire semantics have not been verified.
Keep the eventual adapter thin and use that SDK when available. Do not invent a
new permanent Capsem integration API for this spike.

### User-proposed SDK contract

OpenAPI is the specification authority, with this intended layout:

```text
sdk/
  specification/
  typescript/
  python/
```

Python surface (design sketch, not executable signatures):

```text
from capsem import Hypervisor, VM

Hypervisor(url: str, password: str)
  status()
  list()
  create(name="", profile=..., vcpu=4, memory="8G") -> VM

VM(url: str, password: str, name="", id="")
  exec()
  stop()
  copy_from()
  copy_to()
  fork()
  resume()

Later / not yet implemented:
  rmount()
  rwmount()
  expose(vmport: int, hvport: int, protocol=TCP) -> hypervisor port
  subnet(ip="", netmask=...)
```

The proposed networking direction is a Rust router subprocess carrying guest
network traffic over VSOCK. Preserve that direction: do not add a real guest NIC
or silently use host Docker networking as the Capsem execution path. OpenAPI
describes the SDK control API; guest packet/stream framing over VSOCK is a
separate transport contract to establish with the SDK/router owners. Do not
assume that an HTTP SDK connection itself uses VSOCK.

Mapping to Inspect, to validate once the SDK exists:

| Inspect responsibility | Proposed SDK operation | Contract to establish |
|---|---|---|
| Allocate a sample environment | `hv.create()` | Profile/image selection, readiness, unique identity |
| Execute sandbox command | `vm.exec()` | argv, input, cwd, env, user, outputs, status, deadlines, cancellation |
| Stage/read sample files | `vm.copy_to()` / `vm.copy_from()` | Text/binary fidelity, paths, limits and errors |
| Clean up a sample | `vm.stop()` plus destruction semantics | Stop must not silently stand in for deleting persistent data |
| Reuse an environment template | `vm.fork()` / `vm.resume()` | Optional optimization after fresh-sample isolation is proved |

The proposed surface has no explicit delete/destroy operation. Resolve whether
ephemeral `stop()` destroys state or whether a separate SDK deletion operation
is planned before implementing Inspect cleanup. Also establish how an OCI image
is selected: `profile` currently names a Capsem profile, not an arbitrary image.
These are concrete SDK integration questions, not reasons to introduce another
API here. Mounts, port exposure and subnets are outside the first sample proof.

For the router experiment, prove the complete path:
container traffic -> guest forwarding/namespace plumbing -> VSOCK -> Rust
router subprocess -> existing policy/credential/telemetry owners -> upstream.
The exact ordering and ownership must follow the implemented security boundary;
a new forwarding route must not bypass it. Test allowed and denied traffic,
DNS, CA trust, connection teardown and attribution to the sample/session.

Inspect supports custom sandbox providers, so Docker is not intrinsically required
to integrate Capsem. A Capsem VM could serve as the sandbox directly for tasks
whose dependencies fit a profile. Container support becomes necessary when we
want to consume tasks' existing container environments. Docker/Compose-specific
task definitions require additional compatibility beyond executing an OCI bundle.

References: [Inspect sandboxing](https://inspect.aisi.org.uk/sandboxing.html) and
[custom sandbox providers](https://inspect.aisi.org.uk/extensions-sandboxes.html).
The relevant operations are command execution, file reads/writes, and environment
lifecycle. The adapter must preserve input, working directory, environment,
requested user, stdout/stderr, exit status, timeout and output-limit semantics.
Inspect's evaluation orchestration stays outside the sandbox; only operations
requested through its sandbox interface execute there. This also means model
calls made by that host-side orchestration do not automatically pass through
Capsem's guest network telemetry.

## Historical source baseline (before this change)

- `config/docker/image/kernel/defconfig.arm64` and `defconfig.x86_64` do not
  explicitly enable namespaces, PID/UTS/IPC/network/user namespaces, or cgroups.
  The generated kernel configuration still needs inspection after any rebuild;
  absence from these inputs alone is not proof of a disabled final option.
- `config/profiles/code/apt-packages.txt` contains `util-linux` and `bubblewrap`,
  but no Docker, containerd, runc, crun, or Podman selection.
- `guest/artifacts/capsem-init` does not mount cgroupfs. It redirects guest-local
  traffic through iptables OUTPUT to the DNS/HTTP/HTTPS proxies. Container bridge
  traffic needs separate routing proof; existing OUTPUT rules do not establish it.
- The guest has no real NIC. Adding containers does not require nested hardware
  virtualization: they would use the existing guest Linux kernel.
- The immutable rootfs has a writable overlay. `/root` is normally VirtioFS;
  container layer storage needs a deliberate location and a real storage test,
  rather than assuming overlay-on-overlay or VirtioFS will suit a runtime.

## Executed baseline experiment

Ran the local 0.6.3 CLI against the already-running installed service, using a
fresh disposable `code` session. `capsem status` reported service 0.6.1 and assets
2026.0807.6. These are installed-runtime observations, not proof of the current
worktree's built artifacts. No service restart or profile replacement was done.

Reproduction from the original checkout (adjust binary path for another build):

```sh
python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 90 -- \
  cache/target/cargo/release/capsem run --profile code --timeout 25 \
  'uname -a; ls /proc/self/ns; cat /proc/cgroups; cat /proc/filesystems; for runtime in runc crun docker podman containerd; do command -v "$runtime" || true; done; unshare --mount --pid --fork /bin/true; echo namespace_probe_exit=$?'
```

Observed:

```text
Linux code-7 6.18.43 #1 SMP Fri Aug 7 06:19:14 UTC 2026 aarch64 GNU/Linux
/proc/self/ns: mnt only
cat: /proc/cgroups: No such file or directory
filesystems include ext4, overlay, virtiofs, erofs; no cgroup/cgroup2
no runtime paths printed
unshare: unshare failed: Invalid argument
namespace_probe_exit=1
```

The enclosing diagnostic exits zero because its last command prints the result;
the namespace experiment failed with exit 1. This is negative capability evidence,
not a passing container test. `capsem list` afterwards showed no `code-7` session,
consistent with `run` destroying the temporary session.

The gate digest also records intermittent `glowup.install` failures (3/10 recent
runs). No full gate or release qualification was attempted for this document.

## Subsequent experiments (not implemented)

1. Add network access only after the offline proof. Sharing the guest network
   namespace is a candidate first experiment; prove policy and telemetry through
   the actual proxies, including CA trust inside the container. Bridge networking,
   service-name DNS and multiple environments are separate follow-up work.
2. Map the forthcoming SDK onto Inspect's sandbox lifecycle and execution/file
   operations; run one deterministic Inspect task against it. Use a fixed fixture
   or mock model first so provider credentials and model variance cannot mask
   sandbox failures. Exercise provider conformance tests for the pinned Inspect
   version before claiming compatibility.

`runc` executes OCI bundles; it is not an image registry client or a Docker API
replacement. Image fetching/unpacking and task configuration remain separate work.
See [runc](https://github.com/opencontainers/runc) and
[Docker host networking](https://docs.docker.com/engine/network/drivers/host/).

## Initial effort estimate (historical)

Engineering judgment, not measured delivery commitments:

| Milestone | Rough effort | Largest uncertainty |
|---|---|---|
| One offline OCI container in a Capsem VM | 1–3 engineering days | Kernel/runtime requirements and image rebuild loop |
| Minimal Inspect adapter using the forthcoming SDK | 2–5 additional days after SDK availability | SDK execution, files, cancellation and lifecycle semantics |
| Existing Docker/Compose-based evaluations | Several additional weeks, scope dependent | Builds, image architecture, networking, multiple environments and API assumptions |

The offline milestone is now proved above. Inspect remains the integration
target; an SDK-backed direct-VM provider can be useful before arbitrary container
environments are supported.

## Packaged OCI layer unpacking (ARM64, 2026-09-10)

Both profile package lists now include `umoci`. A deterministic two-layer OCI
fixture proves ordinary and opaque whiteouts, binary bytes, an absolute symlink,
and combined image Entrypoint/Cmd inside the real guest. Unpacking stays inside
the disposable VM; the host registry client never extracts image archives.

The new Ironbank test first failed with exit 127 (`umoci: command not found`).
After rebuilding it reports `umoci version 0.4.7+ds-3+b7` and
`OCI_UNPACK: whiteouts,binary,symlink,entrypoint,cmd=PASS`. Fixture manifest:
`sha256:667bdbc91c490a85bf74d827badcd06623a28b360c4154799bdeca4c78175f86`.

Reproduction, from this worktree (wrap direct commands with the repository's
bounded-command runner):

```sh
just _build-rootfs arm64 code
just _materialize-config
env CAPSEM_RELEASE_BIN_DIR=/Users/elie/git/capsem/cache/target/cargo/release \
  uv run --project build_system --frozen pytest -c build_system/pyproject.toml \
  --rootdir . tests/ironbank/test_oci_unpack.py \
  tests/ironbank/test_oci_container.py tests/ironbank/redis_acceptance.py -q
```

Build `20260910-043343-5f7547-build-assets` passed all ten steps in 6m48s.
The preceding build failed after rootfs creation because manifest generation
included an unrelated, incomplete x86_64 directory. The scoped-manifest fix
retains strict checks for every selected architecture; its evidence is 271
focused Python tests and 18 Rust image-build tests, plus Ruff, Ty and Clippy.

Rebuilt asset SHA-256 identities:

| ARM64 asset | SHA-256 |
|---|---|
| kernel | `b1d8357da3005ef29c9dd2797e83177b85ad25494b13802a78603b2f0b843892` |
| initrd | `1ec23e3842e9e877e3cc37a54c0ac9e56bc584405d24039f678d0ca90f178799` |
| rootfs | `a41d85bb3cfdc624f47be2e80706d7b2d16e5417beebc8546b8514f377cc388e` |

Unpacking, the offline adversarial suite, fresh-session isolation and real Redis
acceptance passed together: five host tests in 12.61s. The added guest-hardening
check and unpacking test then passed together in 3.35s, exercising the existing
doctor checks for immutable guest binaries, immutable rootfs block device and
absence of real NICs. Evidence is under `cache/target/tests/oci-umoci-proof/` and
`cache/target/tests/oci-umoci-hardening/`.

This is ARM64 execution evidence. The x86_64 package list is aligned, but its
new rootfs has not been built or executed. Installed service/profile state was
not replaced. `capsem run IMAGE`, retained OCI caching and TCP publishing are
still open Sprinty work; these tests do not claim those product paths exist.
