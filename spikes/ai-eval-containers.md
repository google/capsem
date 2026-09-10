# Inspect AI / container feasibility spike

Status: baseline investigation complete; container execution is not implemented or proved.

Worktree: `/Users/elie/git/capsem-ai-eval-spike`
Branch: `worktree/ai-eval-containers`
Source baseline: `a6dc86a43`
Sprinty: worktree-local `.sprinty`, item `S01-001`.

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

## Source evidence

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

## Next bounded experiment

1. Extend guest kernel inputs in this worktree for the chosen runtime's required
   namespaces and cgroups. Verify the generated configuration and booted features.
   Select user namespaces and device-controller behavior deliberately; do not
   broadly enable unrelated subsystems to satisfy a Docker checklist.
2. Package a pinned runtime through the profile build rail, mount cgroup v2 in
   guest initialization, and rebuild isolated assets through the normal admin/just
   rail. Do not replace installed assets for a spike.
3. Run one preloaded, architecture-matched OCI bundle offline. Assert a marker,
   separate stdout/stderr, nonzero exit propagation, file round-trip, timeout
   cancellation and cleanup. Record runtime/image digests and kernel identity.
4. Repeat with a second fresh session to prove filesystem isolation. Then test
   resource limits and concurrent samples. Treat a VM as the outer isolation
   boundary, not a container as a replacement for it.
5. Add network access only after the offline proof. Sharing the guest network
   namespace is a candidate first experiment; prove policy and telemetry through
   the actual proxies, including CA trust inside the container. Bridge networking,
   service-name DNS and multiple environments are separate follow-up work.
6. Map the forthcoming SDK onto Inspect's sandbox lifecycle and execution/file
   operations; run one deterministic Inspect task against it. Use a fixed fixture
   or mock model first so provider credentials and model variance cannot mask
   sandbox failures. Exercise provider conformance tests for the pinned Inspect
   version before claiming compatibility.

`runc` executes OCI bundles; it is not an image registry client or a Docker API
replacement. Image fetching/unpacking and task configuration remain separate work.
See [runc](https://github.com/opencontainers/runc) and
[Docker host networking](https://docs.docker.com/engine/network/drivers/host/).

## Effort estimate

Engineering judgment, not measured delivery commitments:

| Milestone | Rough effort | Largest uncertainty |
|---|---|---|
| One offline OCI container in a Capsem VM | 1–3 engineering days | Kernel/runtime requirements and image rebuild loop |
| Minimal Inspect adapter using the forthcoming SDK | 2–5 additional days after SDK availability | SDK execution, files, cancellation and lifecycle semantics |
| Existing Docker/Compose-based evaluations | Several additional weeks, scope dependent | Builds, image architecture, networking, multiple environments and API assumptions |

Recommendation: keep Inspect as the integration target and prove one offline
container as the packaging experiment. Do not make a complete Docker-compatible
engine a prerequisite for the first Inspect task. An SDK-backed direct-VM
provider can be useful before arbitrary container environments are supported.
