# Inspect AI / container feasibility spike

Status: offline OCI execution proved on rebuilt ARM64 assets in real Apple VZ
VMs. x86_64 kernel compilation passed; no x86_64 runtime proof.

Worktree: `/Users/elie/git/capsem-ai-eval-spike`
Branch: `worktree/ai-eval-containers`
Source baseline: `a6dc86a43`
Sprinty: worktree-local `.sprinty`, baseline `S01-001`, implementation `S02-001`.

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
