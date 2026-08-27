---
name: release-process
description: Capsem's release process: orthogonal binary/profile CI, signing, notarization, channel deployment. Use for release commands, failures, manifests, or anything affecting what ships.
---

# Release Process

## Manifest Authority

The selected manifest is the bible: if an artifact is not recorded in it, it
does not exist for a release lane. Fetch the mutable manifest fresh after the
channel lock is acquired. Large immutable inputs may be cached only under the
artifact digests already recorded in that manifest, independently of channel,
and every cache hit must be digest-verified before use. Cache contents,
filenames, GitHub Releases, and prior workflow runs never add membership.

## Governing contract

Read `tmp/release-spec.md` before changing release commands, manifests,
workflows, test composition, artifact publication, or update behavior. It is
the normative contract when older repository text disagrees.

## Reference routing

- Read `references/qualification-and-test-composition.md` before changing
  public release commands, plan composition, candidate/source guards, sandbox
  or egress behavior, fail-stop ordering, shared test modules, artifact
  staging, `ProfileContent`, or `--force` (CI-only changes; never shipped bytes).
- Read `references/lane-workflows.md` before changing channel locking, preview
  deployment, profile/binary ownership, nightly sequencing, staged activation,
  base-image materialization, or corporate authoring.
- Read `references/release-graph.md` before changing graph generation, channel
  membership, manifest authoring, immutable identity, or public activation.
- Read `references/installation-verification-and-retry.md` before changing
  evidence/integrity rules, native package acceptance, installed status,
  artifact retention, live validation, retry, diagnostic continuation, or
  Cloudflare deployment checks.
- Read `references/ci-invariants.md` before editing release workflows,
  platform/toolchain/scanner setup, Docker/storage behavior, package rails, or
  hosted-runner capacity. It contains the hard-won parity lessons.
- Read `references/apple-signing.md` before touching signing, notarization,
  certificates, Tauri keys, Apple agreements, or release CI secrets.
- Read `references/post-release-verification.md` after any public deployment
  and before changing the public installer, transition, or glow-up proof.
- Read `references/versions-and-commit-discipline.md` before changing release
  notes, binary/profile versions, compatibility bounds, profile revision
  advancement, release-set identities, or release commit practice.

## Core release contract

Capsem has exactly two release-facing commands:

```bash
just release-binaries <channel> <source-commit>
just release-profile <channel> <profile> <source-commit>
```

They freeze and validate a committed full lowercase commit on local `main`,
publish its immutable `capsem-source-<commit>` ref, and dispatch the hosted
lane that qualifies its owned artifact family before publication. They do not
consume a developer-machine candidate journal. Do not add a third release
command or dispatch the workflows directly.

`just test-clean <commit>` remains the exceptional cold complete diagnostic.
Its resumable journal is useful for reproducing stale-cache and physical Mac
defects, but it is not publication authority and agents must not run it after
each source edit.

A failed run archives its journal and retains the full-SHA prefix. A repeat may
use only its deepest proven frontier; the child records carried steps and a
content-addressed parent. Manual continuation must match the derived prefix,
frontier, and carried set. Reuse-only success cannot extend the chain.

The complete candidate remains inside the host-kernel network boundary.
Bubblewrap on Linux and Seatbelt on macOS provide loopback only; the one-time
authenticated egress helper serves only marked advisory queries, fresh
manifest/version-ref resolution, remote-main validation, source-ref
publication, and final dispatch. Every brokered command remains runner-guarded
and journaled. Never widen the whole release because one edge needs network.

Candidate and both release commands accept only the enforcing sandbox mode;
`off` and `report` are diagnostic modes for incomplete modules and can never
produce complete qualification evidence.

Exceptional local `just test-clean` rebuilds every package/profile and runs
the six release modules: `_test-fast`, `_test-static`, `_test-artifacts`,
`_test-functional`, `_test-glowup`, and `_test-release-contracts`.

Release CI saves construction time, never test quality. The binary lane builds
packages and digest-resolves profiles; the profile lane builds exactly one
channel/profile and digest-resolves the selected package. Both stage the exact
complementary family into the shared modules; source-built substitutes and
ambient release-variable assertion forks are forbidden. `just fast-test` is
developer feedback and the exact `_test-fast` module, not qualification.

Assets and materialized configuration travel as one `ProfileContent` root.
Package construction, Debian proof, macOS Tart/physical-VZ proof, and final
install/glow-up must derive both paths from that one value and validate it
before Docker or Colima. Release CI stages raw manifest inputs into the paired
root on the host; the sealed proof never rematerializes them or falls back to
checkout `assets`/`target/config` selectors.

Binary and profile workflows share the exact
`capsem-release-${{ inputs.channel }}` lock from fresh manifest read through
deployment. Stable and nightly remain independent. The binary lane may mutate
only package/per-binary/host-SBOM/existing-attestation fields and never builds a
profile. The profile lane may mutate only one channel/profile and never builds
a package. Binary inventory is nested under its owning package. Profiles own
their config, images, software inventory, OBOM/evidence. `capsem-admin` is the
sole first-party and corporate manifest/profile
author; corporations never build or mutate Capsem-owned binaries or channels.

A legacy public graph is never inferred dead from a 404. Use only the exact,
config-owned retirement rail in `references/release-graph.md`, then publish the
replacement profile before the binary lane activates a new package cohort.

If a profile needs newer code, publish its immutable bytes once as staged
source state, then run the binary command. That lane resolves the staged
profile by digest, proves the complete pairing, and activates only after full
functional, native-install, Winterfell/MCP, IronBank, doctor, and glow-up
success. Neither artifact family is rebuilt twice; the prior public working
pair survives every failure.

Native installation proves function, not existence. macOS CI owns signing,
notarization, stapling, exact-package installation, and structural checks;
Linux owns exact native `.deb` installation and the guest shell where KVM is
available. Local Apple Silicon `just test-clean` owns that VZ proof. Neither platform
boundary substitutes for another, and skipped or inspect-only checks do not
count.

Linux binary qualification includes the public-before to candidate-after
self-update; it cannot depend on new service code because the previous service
launches the first package transition. Candidate `DEBIAN/preinst` detects its
`dpkg` inside `capsem.service` through `/proc/self/cgroup` and preserves the old
cohort until exact manifest activation requests the managed restart. Ordinary
package replacement still stops the unit and retires stale helpers.

The exact verified `assets/manifest.json` remains the installed source of truth
byte-for-byte. `assets/manifest-metadata.json` is its only metadata sidecar;
runtime may derive an in-memory boot view. CLI and UI consume the same
`GET /system/status` contract and must not synthesize publication state.

A red release lane stops publication. Fix forward without moving tags or
history, then invoke the owning release command for the corrected commit.
`just test-clean <commit>` may reuse its diagnostic journal when reproducing a
local-only defect; release commands reject continuation flags and qualify in
their hosted lanes.

## Version and commit essentials

Keep user-visible binary release notes under `## [Unreleased]`. They are
bookkeeping, not a qualification prerequisite: the immutable version tag is
the release event, and the GitHub release name records the full qualified
source commit. Binary and profile versions are orthogonal strict semver.
`min_capsem_version` and `max_capsem_version` bound the binary, not a profile's
own revision.
`parse_profile_revision` rejects non-semver revisions; `ensure_revision_advances`
rejects non-advancing publication. Mixed sets use `profiles-<hash>`.

Stage explicitly with conventional commits; never stage release secrets.
