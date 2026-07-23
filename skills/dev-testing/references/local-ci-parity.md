# Local/CI Execution Parity (Ironbank parity rule)

Reference for /dev-testing: every portable release gate must be owned by just test. Read before editing any release workflow, gate recipe, or CI job.

## Local/CI execution parity

### Ironbank parity rule

The Ironbank parity rule is that every portable release gate must be owned by
`just test`. A release candidate is not qualified until the complete
`just test` passes locally and exact-SHA CI runs that same recipe successfully.
Specialized CI jobs may provide faster feedback or platform evidence, but they
must call the same checked-in recipe or script already exercised by
`just test`; they cannot be the only owner of a portable requirement.

Treat `just test` as a strict superset, not merely a collection of similar
assertions. Anything a CI workflow builds locally-portably must be built and
tested by `just test` through the same production primitive. CI is allowed to
run only the slice it needs; the local canonical gate is not allowed to omit
that slice. A workflow-only build, even when another test validates its input
schema, is an Ironbank violation because the produced artifact is unaccounted
for.

Tool and scanner parity is part of artifact parity. Pin release-only generators
to one exact version across the local host-builder image, builder default, and
workflow installer; a contract test must reject `@latest` and mismatched pins.
Scan the extracted exported guest filesystem with the exact invocation proven
against the complete Capsem image. Do not change scanner modes from a tiny
fixture alone: cdxgen 12.7.0 `rootfs` mode passes a minimal Debian fixture but
emits Debian's lowercase `sendmail` spelling, which its own CycloneDX schema
rejects. The qualified rail runs `rootfs --no-validate`, normalizes that known
SPDX spelling and all build-host/temp-path nondeterminism, removes the pinned
scanner's colliding cryptographic-asset subset, and then runs the paired strict
schema validator. Never use cdxgen `os` mode for a staged rootfs: it inventories
the live build host even when a directory argument is present. Require an
`exported-rootfs` scope marker, Debian guest components, no osquery categories,
and byte-identical output from two scans of one rootfs. Bound successful scanner
output while preserving captured diagnostics on failure.

Cross-architecture artifacts do not imply cross-architecture helper tools.
Every Docker helper that post-processes an exported artifact must pass an
explicit host-native `--platform`; otherwise an arm64 guest build on x86 CI can
silently run `mkfs.erofs` under QEMU, while a Mac may exercise a different path.
Test the x86_64/amd64 and arm64/aarch64 host aliases and reject unknown hosts.
Capture verbose successful helper output so per-file progress cannot starve a
runner. A qualification runner blackout during an expensive tool is a missing
local/CI resource contract, not permission to rerun unchanged.

Run `scripts/check-hardcoded-release-selections.sh` at the start of `just test`.
This source guard is a release contract, not a style check: user-facing and
profile-scoped requests must obtain profile ids from arguments or the installed
catalog, package rails must materialize the catalog, native installers must use
packaged manifest metadata without a stable/nightly fallback, and exact-SHA
qualification must also match the requested channel. Extend the guarded profile
terms during renames; keep `code`, `co-work`, `cowork`, `terminal`, `termional`,
and `gui` until every migration path is complete.
Because it runs before the expensive test stages, the guard must use only
declared bootstrap dependencies. Its scanner is Python-standard-library
only and has an executable regression with `rg` absent; never reintroduce a
developer-machine-only search command into this fail-fast boundary.

HTTP-client behavior is part of local/CI parity. A localhost fixture returning
200 does not qualify a reader that will fetch `release.capsem.org`: exercise the
production reader against an adversarial local edge that rejects Python's
default `urllib` identity, and execute it once against the live public manifest
before the full gate. Public release readers must send an explicit Capsem user
agent, fail loudly on malformed JSON or an unsupported manifest shape, and be
covered by the fail-fast source guard so a bare `urlopen(url)` cannot return.
Test both the legacy VM-asset manifest and the public release graph; they are
distinct schemas even when they share the name `manifest.json`.

This includes workspace/runtime tests, Rust and Python coverage floors,
`capsem-doctor` and Ironbank acceptance, benchmarks, artifact completeness,
frontend/docs/marketing/release-site validation, and the Docker/systemd Linux
package install and guest-shell proof. It also includes the full profile-owned
VM asset matrix: `just test` calls `just _gate-assets`, which rebuilds every
checked-in profile for arm64 and x86_64 through `just _build-kernel` and
`just _build-rootfs`, validates the release payload and manifest, and boots each
host-architecture result to a real guest-shell marker. Truly non-portable
boundaries remain explicit final gates: Apple signing/notarization, hosted KVM,
and Cloudflare publication. Apple VZ is proven by the complete local gate on
the exact clean candidate before qualification.

Every portable release-critical CI path must be executable locally through the
same production entrypoint. Do not create a local lookalike that merely checks
similar commands. If CI calls a `just` recipe or checked-in script, local proof
must call that same recipe or script; if a requirement is implemented as a
shared shell function, both paths must execute that function.

Generated release graphs and their materialized profile artifacts must come
from the same source snapshot. The local gate qualifies uncommitted candidate
bytes from the worktree; it must never generate descriptors from the worktree
and then fetch artifact bytes from `HEAD`. Production release workflows keep
using an immutable git ref. Guard both modes with functional tests.

Release graph contract tests must use deterministic prepared assets unless a
test explicitly owns full-blob hashing. Never point multiple module fixtures
at the checked-in multi-gigabyte `assets/` tree just to test JSON or HTML.
Asset-build tests prove that one streaming pass records BLAKE3 and SHA-256;
channel tests prove complete manifest digests avoid reopening remote blobs,
historical releases cannot resolve through current-file paths, and local copy
mode fails closed on byte mutation. Use an unreadable/directory blob or an
instrumented open counter as evidence; a timing threshold alone is not a gate.

Artifact accounting is literal: macOS-local `just test` builds the real
release-mode `.pkg`, builds both Linux release-mode `.deb` architectures, and
runs the production host-SBOM generator over those exact packages. Generated
settings are regenerated under a before/after idempotence gate. A fixture-only
package test or a generator source inspection does not account for the artifact
that a release workflow will publish.

Run Linux-only build, doctor, package, and service prerequisites in Docker from
`just test` whenever the host is macOS. Match the CI architecture, command
names, environment variables, permissions, and service manager as closely as
the container permits. Add a contract test that ties the workflow command to
the local entrypoint, plus an executable container regression for the failed
requirement. A CI-only failure is evidence of a missing parity gate: add the
local reproducer before rerunning CI.

Linux test containers must run the test process as a non-root user unless the
CI process is also root. Root invalidates permission-denied regressions and is
therefore a parity failure, not a harmless container implementation detail.

The canonical gate also has a runtime budget. Measure full local and CI stage
durations and keep meaningful headroom below the runner's observed lifetime;
the workflow's declared timeout is not proof that the host will live that long.
Disk headroom is part of the same budget. Large immutable packages and VM
blobs that are already present in the candidate workspace must use
hardlink-first same-filesystem staging on both macOS and Linux, with a tested
cross-filesystem copy fallback. Add a constrained-disk executable regression
that makes an accidental full copy fail with `ENOSPC`; a source assertion or
an unconstrained happy-path run cannot guard a multi-gigabyte late-stage copy.
Record disk use before every expensive artifact lane and before the final
install/glow-up tail so capacity failures happen before hours of qualification.
Parallel Docker gates also own a daemon-space preflight: measure free space,
reclaim only unused builder cache when below the documented reserve, and fail
before launching lanes if the reserve remains unavailable. Preserve each lane's
complete log and wait for the logging pipeline itself, so failure diagnostics
cannot race a still-flushing process substitution.
Two runs terminating at the same wall-clock age are a deterministic budget
failure until disproved, not random infrastructure. Before another CI attempt,
reproduce the expensive rail locally and remove the critical-path bottleneck.
Parallelize only independent work with isolated workspaces, Docker tags, output
roots, and cleanup ownership, and add a regression that asserts that isolation.
Docker tags are daemon-global across worktrees. Automatic gates must never run
`docker image prune --all`/`-a`, and an internal primitive used by concurrent
lanes must not invoke garbage collection. A newly tagged cached image can have
an old creation timestamp and be deleted by age-filtered `prune -a` from another
lane or checkout. Run cleanup only at an owning outer boundary, prune dangling
images and unused builder cache, and emit captured Docker stderr on failure.
Any preflight that mutates the container VM or daemon must have a hard
wall-clock timeout and an executable test that waits for process completion.
Printed success output is not completion proof. In particular, never set the
Colima VM clock from a privileged Docker container: `date` can exit while the
Docker client remains blocked in cleanup. Use the bounded host-side Colima
clock primitive.

When an unavoidable platform boundary prevents local execution, name it in the
release skill and retain the nearest deterministic local proof. Hardware and
external-service gates still require exact-SHA CI evidence. macOS VZ behavior
requires the complete local `just test` on the exact clean candidate, while
hosted CI owns final package signing, notarization, installation, and
structural verification. Never silently omit either gate because one
environment cannot run the other environment's proof.
