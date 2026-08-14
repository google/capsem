# Capsem Binary, Profile, Manifest, and Channel Release Specification

## Status

This document is the normative product and CI specification for separating
Capsem binary releases from profile releases while preserving complete
compatibility, package, integrity, update, and deployment proof.

It defines what the release system must guarantee. It does not prescribe the
final workflow filenames, storage provider, tag syntax, or implementation
language unless a requirement explicitly depends on one of them.

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHOULD**, **SHOULD NOT**,
and **MAY** are normative.

When an older repository document conflicts with this specification, this
specification governs. The release implementation, tests, `AGENTS.md`, checked-in
skills, and developer documentation MUST be changed together so two release
models are never simultaneously described as authoritative.

## 1. Purpose

Capsem has two independently moving product dimensions:

1. The Capsem host binary and its native packages.
2. Channel-scoped profiles, where each profile contains its complete VM asset
   set and configuration.

The release system MUST let these dimensions move independently. A Capsem bug
fix must not force profile rebuilding, large asset transfers, or VM
invalidation. A profile change must not force a Capsem binary rebuild when the
currently selected binary remains compatible.

This independence must not reduce confidence. Local development qualification
MUST exercise the complete construction pipeline. CI release jobs MUST build
only the component being released, but MUST test that candidate against the
immutable, already-built complementary components with which it will operate.

The resulting rule is:

> Local qualification rebuilds the whole system. CI release lanes build only
> what they own and prove compatibility against immutable outputs owned by the
> other lanes.

The first-party operator surface has exactly two release commands:

```text
just release-binaries <channel> <source-commit>
just release-profile <channel> <profile> <source-commit>
```

There is no generic release command and no combined release command. When a
profile requires new Capsem code, the operator runs `release-profile` first and
`release-binaries` second. The profile assets are built once, remain inactive
while incompatible with the public binary, and are pulled as immutable inputs
by the following binary release.

## 2. Goals

The release system MUST provide all of the following:

- Independent versioning and publication of Capsem binaries and profiles.
- Profiles scoped to channels, with no assumption that the same profile exists
  in every channel.
- Independent channel/profile instances when a profile name exists in more
  than one channel.
- Daily, scheduled nightly rebuilds of the binary family and every selected
  profile/asset family from one full commit snapshot of current `main`, through
  their independent public commands rather than publication after every push.
- Manually initiated stable binary and profile publication.
- Targeted profile publication for exactly one channel/profile pair.
- An ordered `release-profile` then `release-binaries` path when a new or
  changed profile requires a new Capsem binary, without rebuilding the profile.
- One shared per-channel concurrency lock covering both release commands from
  manifest read through generated-distribution deployment.
- Exclusive manifest and profile authoring through `capsem-admin`.
- Corporate manifests and profiles authored through `capsem-admin` with the
  profile source commit, without requiring corporations to build Capsem binaries.
- Regular manifest polling and automatic, verified Capsem updates.
- Native package proof through the installed-product glow-up suite.
- Integrity, compatibility, provenance, and rollback safety before any channel
  becomes visible to installed Capsem instances.
- Reusable CI modules shared by nightly, stable, profile-only, combined, and
  corporate validation paths.
- Named contract tests that prove each CI lane's write boundary.

## 3. Non-goals

The following are explicitly outside the intended model:

- Rebuilding every profile because a Capsem binary changed.
- Rebuilding Capsem binaries because a compatible profile changed.
- Treating an asset as an independently authored release unit outside its
  profile.
- Assuming that a profile exists in all channels.
- Implicitly propagating a profile update from one channel to another.
- Publishing a binary on every push to `main`.
- Providing corporations with a path to build or replace official Capsem
  binaries.
- Supporting hand-written manifests or an alternate manifest/profile authoring
  tool.
- Relying on users to manually download releases or manually keep components
  synchronized.
- Treating a successful build, checksum calculation, package expansion, or
  mocked install as proof that a release works.
- Duplicating build, package, profile, manifest, glow-up, or deployment logic
  across workflow files.
- A standalone commit-SHA qualification workflow or checker separate from the
  two release commands. The explicit commit argument is part of each command.
- A second release-history, transaction-state, recovery-state, or result
  document beside the manifest.
- A caller-authored profile tag or publication identity.

## 4. Terminology

### 4.1 Capsem binary

The compiled Capsem host software produced and released by the Capsem project.
A binary release includes the executables and the metadata needed to identify
and verify them.

Only the Capsem project builds and publishes official Capsem binaries.

### 4.2 Native package

A platform-specific delivery artifact containing a Capsem binary cohort, such
as a macOS or Linux package. Package existence is not installation proof. The
exact publishable package must be installed and exercised before publication.

### 4.3 Binary inventory

The per-package inventory of installed Capsem executables, versions, paths,
hashes, and evidence references. Binary inventory belongs to the binary release
lane.

### 4.4 Host SBOM and host attestations

The software bill of materials and provenance evidence for Capsem host
packages and binaries. These belong to the binary release lane.

### 4.5 Profile

The complete authorable VM definition. A profile includes its configuration,
images, evidence, revision, integrity metadata, and every asset required to
instantiate and run it.

A profile is not a pointer to a separately mutable bag of assets. Changing a
profile means rebuilding the affected channel/profile and its contained assets.

### 4.6 Profile assets

All immutable build outputs contained in a profile, including its VM images and
profile-owned configuration and evidence. Assets are outputs of a profile
build; they are not independently authored release units.

### 4.7 Channel

A named update stream defined by a manifest. Examples include `nightly`,
`stable`, and corporation-defined channels.

A channel selects a Capsem binary policy and contains zero or more profiles.
Channel membership is explicit.

### 4.8 Channel/profile

The independently releasable profile instance identified by the ordered pair:

```text
(channel name, profile name)
```

For example, `nightly/default` and `stable/default` are independent instances.
They may contain identical bytes at one moment, but neither is an alias for the
other and neither may be mutated as a side effect of updating the other.

### 4.9 Profile tag

The immutable publication identity derived by `capsem-admin` from the selected
channel, profile, and declared profile revision. Callers MUST NOT compose or
supply the identity. `capsem-admin` MUST reject reuse of an existing identity
for different bytes.

### 4.10 Manifest

The authoritative update and integrity document authored through
`capsem-admin`. A manifest defines channels and, for each channel:

- The selected Capsem binary version or resolution policy.
- Any minimum and maximum Capsem version constraints.
- The profiles belonging to that channel.
- The selected revision of each channel/profile.
- Integrity metadata for packages, binaries, profiles, and profile assets.
- Compatibility relationships needed to prevent invalid binary/profile
  combinations.
- Evidence needed to verify the origin and contents of referenced artifacts.

A manifest is not merely an asset list. It is the authoritative description of
the complete, valid release graph that installed Capsem instances poll.

The manifest is the bible: if an artifact is not selected by the manifest, it
does not exist for release, update, test, cache, or boot purposes. A cache,
release attachment, filename, directory listing, previous workflow, or channel
convention MUST NOT add membership or substitute bytes.

Mutable manifests MUST be fetched fresh at the owning transaction boundary.
Large immutable artifacts MAY be cached, but each cache entry MUST be addressed
directly by a digest already recorded in the selected manifest. Cache identity
MUST be independent of channel. Every restored byte MUST be verified against
the manifest's recorded size and digests before use; corrupt entries MUST be
discarded and fetched again. Architecture filtering MAY reduce transfers only
by selecting the requested architecture rows from that same manifest.

### 4.11 Source manifest

The authoring state owned by `capsem-admin` from which validated and deployable
manifest output is generated. Source manifests MUST NOT be rewritten by the
channel deployment lane.

### 4.12 Generated public distribution

The complete deployable output generated from validated manifest state and
immutable release artifacts. It is the only input accepted by the channel
deployment lane.

### 4.13 Existing artifact

An immutable binary, package, profile, or profile asset that was built and
qualified previously and is available as a read-only compatibility input.

“Existing” means selected by the manifest and verified by immutable version and
digest. A channel or corporate policy MAY say `latest`, but `capsem-admin` and
the updater MUST resolve it to concrete immutable package identities before
testing or mutation. It never means a mutable local cache entry or an
unverified download.

### 4.14 Candidate artifact

An immutable staged output produced by the current release run but not yet
activated in a public channel.

### 4.15 Activation

The final operation that changes the generated public distribution observed by
polling Capsem installations. Uploading immutable candidate bytes is not, by
itself, activation.

### 4.16 Glow-up

The installed-product test suite that proves real native packages and the
automatic update path work across required state transitions. Glow-up is not a
build smoke test and not a collection of parser tests.

## 5. Authoritative object model

The release graph has this hierarchy:

```text
Manifest
├── Channel A
│   ├── Capsem binary selection and compatibility policy
│   ├── Profile A1
│   │   ├── configuration
│   │   ├── images/assets
│   │   ├── integrity metadata
│   │   ├── evidence
│   │   └── revision
│   └── Profile A2
│       └── ...
└── Channel B
    ├── Capsem binary selection and compatibility policy
    └── Profile B1
        └── ...
```

The following invariants are absolute:

1. The manifest defines channels.
2. A channel owns its profile membership.
3. A profile may be absent from any channel.
4. A profile name may exist in multiple channels.
5. Each channel/profile pair is independent.
6. A channel/profile update MUST NOT alter any other channel/profile.
7. A profile contains its assets.
8. A profile change MUST rebuild that channel/profile's complete required asset
   set.
9. A binary release MUST NOT rewrite profile data.
10. A profile release MUST NOT build or rewrite Capsem binaries or packages.
11. All manifest and profile authoring MUST go through `capsem-admin`.
12. Installed Capsem instances update by polling manifests and verifying the
    selected graph.

## 6. Version and compatibility model

### 6.1 Independent versions

Capsem binary versions and profile revisions are orthogonal. Neither version
may be inferred from the other.

A manifest connects them by declaring which binary selection and profile
revisions form a valid channel state.

### 6.2 Binary selection

A channel MUST select its Capsem binary using one of the policies supported by
`capsem-admin`, such as:

- An exact pinned Capsem version.
- A controlled “latest” policy resolved to a concrete immutable version during
  manifest generation or update evaluation.

The deployed manifest MUST always resolve to immutable, verifiable artifacts.
The word “latest” must never weaken integrity or make a release irreproducible.

### 6.3 Compatibility bounds

The manifest MUST carry a minimum version and MAY carry a maximum version when
an upper bound is actually known. An omitted maximum is unbounded. The release
system MUST NOT invent a maximum merely to make validation appear complete.

Compatibility validation MUST determine whether:

- The selected binary can consume the selected profile revision.
- Every profile in the channel can run with the selected binary.
- A candidate binary remains compatible with the existing profiles it is
  expected to preserve.
- A candidate profile remains compatible with every existing binary version
  that the channel continues to declare supported.

An empty or contradictory compatibility intersection is a hard validation
failure.

### 6.4 Release compatibility cohort

For a profile release, CI MUST pull and test the channel's currently selected
released binary. It MUST NOT rebuild a binary for a compatible profile release.
When a profile intentionally requires newer code, the existing binary/profile
pair is rejected by the declared minimum and the profile remains staged until
`release-binaries` supplies and tests the required binary.

For a binary release, CI MUST pull and test every channel/profile revision in
the resulting selected channel, including every profile previously staged by
one or more `release-profile` runs. It MUST NOT rebuild any of those profiles.
The composed transition is the complete staged profile set; it MUST NOT impose
an artificial one-profile activation limit.

These release gates prove the graph being activated. They do not claim that
every historical binary ever published remains supported.

### 6.5 Compatibility failure behavior

If a profile requires a binary newer than the selected channel binary, that
profile MUST remain staged and inactive until the composed profile-then-binary
flow completes.

The system MUST NOT temporarily publish an incompatible pair and rely on
polling order to repair it later.

## 7. Authoring model

### 7.1 Exclusive authoring tool

All first-party and corporate manifest/profile authoring MUST occur through
`capsem-admin`.

There MUST NOT be:

- A hand-written production manifest path.
- A second manifest generator in CI.
- A workflow-specific manifest editor.
- A script that bypasses `capsem-admin` for profile membership, bounds,
  versions, revisions, or digests.
- A corporate-only shortcut with different validation semantics.

Reusable CI modules MUST invoke the same `capsem-admin` production entrypoints
used by human administrators.

### 7.2 First-party authoring

The Capsem project uses `capsem-admin` to:

- Define first-party channels.
- Set the binary version or policy for each channel.
- Define channel-specific profile membership.
- Build targeted channel/profile instances.
- Set compatibility bounds.
- Generate integrity and evidence metadata.
- Generate validated manifest and public distribution inputs.

First-party release entrypoints are deliberately asymmetric:

- `just release-profile <channel> <profile> <source-commit>` calls `capsem-admin release`
  directly for the selected channel/profile. The command gives the workflow a
  unique correlation identity, discovers that exact run, and waits for its
  terminal status before returning.
- `just release-binaries <channel> <source-commit>` calls one checked-in, adversarially tested
  binary-release script.

Dispatch acceptance is not release completion. A successful profile command
MUST mean that its exact profile workflow succeeded; it MUST NOT return after
merely adding an unidentified run to the same-channel queue. This is what makes
sequential profile and binary automation safe even when another release is
already pending.

The first release into a missing first-party channel MUST still obey this
surface. The serialized profile workflow MAY ask `capsem-admin release` to
initialize the selected channel from the verified official package cohort of
the other existing first-party channel. That initial source manifest MUST:

- Be created only after the selected channel lock is held.
- Contain the selected channel name and the verified existing package cohort.
- Contain explicit empty profile membership.
- Copy no profile from the donor channel.
- Mutate neither the donor channel nor its source manifest.
- Be rejected if the public channel catalog already claims that the selected
  channel exists.

The selected profile is then authored through the normal `capsem-admin release`
path. Bootstrap is not a third public release command, a generic authoring
shortcut, or permission to relabel a release graph from another channel.

The explicit empty-membership bootstrap manifest is serialized authoring state.
It MUST NOT be deployed, installed, described as a working pairing, or given
synthetic Doctor or Winterfell evidence. For the channel's first profile, there
is no public-before profile transition to claim. The completed candidate pairing
MUST instead pass a genuine native fresh install, full Doctor, installed
Winterfell, tamper and incompatibility polling, and preservation proof before
the channel may become public. Once the channel has an activated profile,
subsequent profile releases MUST additionally prove the real public-before to
candidate-after profile transition.

There MUST NOT be a generic release command, a combined command, or a public
collection of internal release stages.

### 7.3 Corporate authoring

Corporations use `capsem-admin` to author:

- Their own manifest.
- Their own channels.
- The profiles belonging to those channels.
- Their profile configuration and contained assets.
- Their selection of official Capsem binaries.
- Exact binary pins or an allowed latest-version policy.
- Their compatibility bounds and integrity metadata.

Corporations MUST NOT:

- Build official Capsem binaries.
- Replace official Capsem package bytes.
- Modify first-party Capsem channels.
- Modify first-party profile instances.
- Publish into first-party namespaces.
- Bypass `capsem-admin`.

Corporate support therefore separates ownership cleanly: the Capsem project
publishes binaries; a corporation authors its manifest and profiles and refers
to compatible official binaries.

## 8. Local qualification

### 8.1 Purpose

Local `just test` is the comprehensive construction and integration gate. It
exists to catch incompatibilities throughout the complete pipeline, including
breakage outside the component currently being edited.

### 8.2 Required scope

Local `just test` MUST exercise 100% of the configured release pipeline,
including:

- Building Capsem binaries.
- Building native packages.
- Building the complete configured profile catalog.
- Rebuilding the assets contained in those profiles.
- Generating manifests through `capsem-admin`.
- Validating channel membership and compatibility bounds.
- Verifying artifact integrity and evidence.
- Installing the real locally built packages.
- Booting representative real VMs from the rebuilt profiles.
- Exercising automatic manifest polling and update planning.
- Exercising binary-only, profile-only, and combined update compatibility.
- Running the complete package glow-up suite where the local platform supports
  it.
- Running all other correctness, security, integration, and regression gates
  included in the canonical local qualification command.

The exact internal steps may evolve, but the public meaning of `just test`
remains “construct and verify the whole system.”

`just test <source-commit>` selects one canonical full lowercase commit that is
already prepared, committed, and reachable from local `main`. It MUST
materialize and qualify an independent detached repository at a prefix named
by that full commit. The mutable outer checkout and branch are not
qualification inputs and MAY advance while it runs. Each release command MUST
require and revalidate that complete exact-commit journal before any source
publication or dispatch. It MUST NOT repeat the local candidate, edit tracked
source, create a preparation commit, or push `main` after the proof.

The runner MUST archive each exact-source attempt journal independently of
ordinary run rotation. A repeated `just test <source-commit>` MUST return
ordinary success immediately when a complete journal still validates, naming
the original run ID, path, and content digest. A failed attempt MAY resume only
from its retained full-SHA prefix and the deepest frontier whose graph-derived
ancestors are covered by that exact journal. The new attempt MUST record a
content-addressed parent and every carried step. Recursive coverage of every
declared step is required before the chain becomes complete. A reuse-only run,
manual marker, skill, or guessed continuation MUST NOT become evidence.

Before starting Docker/Colima, bootstrap, package, profile, asset, or VM work,
`just test` MUST run one checked-in private `_test-fast` module. That same
module MUST be called independently by ordinary CI, both release workflows,
and `just smoke`. It MUST own all cheap deterministic failures, including YAML
and workflow parsing, Python/shell/JSON/TOML syntax, generated-file drift,
source and release contracts, Rust Clippy, Python lint and type checks,
JavaScript type/test/build checks, and blocking Rust, Python, and JavaScript
dependency-vulnerability audits. These checks MUST NOT be duplicated as a
smaller smoke-only or workflow-only approximation.

`just smoke` remains a public developer-feedback command. Its use of
`_test-fast` does not make it release qualification: only `just test` adds the
complete construction, artifact, VM, functional, native-install, and glow-up
proof required before either release command may dispatch.

### 8.3 Local rebuilding is intentional

Local asset rebuilding is not waste to optimize away. It is how local
qualification proves that current source inputs still construct compatible
binaries, profiles, assets, packages, and manifests.

This rule applies to local qualification. It MUST NOT be used as justification
for rebuilding every profile inside a selective CI release lane.

## 9. CI architecture

### 9.0 Shared per-channel transaction

The binary and profile entry workflows MUST both declare:

```yaml
concurrency:
  group: capsem-release-${{ inputs.channel }}
  cancel-in-progress: false
```

The workflow-level lock MUST be acquired before the selected channel's source
manifest is fetched and retained through input resolution, construction,
testing, manifest mutation, generated-distribution assembly, and production
deployment. A queued run MUST fetch the manifest only after it owns the lock.

Binary and profile releases for the same channel, and two profile releases for
the same channel, therefore run in order. Different channels MAY run in
parallel. Production channel deployment MUST be invoked only by a parent
release workflow holding this lock.

### 9.1 Selective construction, complete relevant proof

CI MUST distinguish between:

- What the current lane is allowed to build or write.
- What immutable complementary artifacts it must read and test.
- What compatibility claim the resulting manifest will make.

Each release lane MUST build only its owned candidate artifacts. It MUST still
exercise the complete real compatibility and installed-product paths relevant
to the release.

Examples:

- A binary release builds packages but consumes existing profiles read-only.
- A profile release builds one channel/profile but consumes existing binaries
  read-only.
- A composed release builds a profile first, then a binary, while preserving
  both lanes' separate write scopes.

Generated-distribution assembly MUST preserve every non-selected channel from
its deployed public manifest and exact referenced bytes. It MUST NOT rebuild,
reauthor, normalize, or require migration of that preserved channel merely
because another channel is being released. Legacy referenced bytes MAY be
copied from the current public distribution after their recorded size and
digests are verified; this is preservation, not authoring. Only the selected
channel may be generated from candidate source state.

### 9.2 No YAML business logic

Workflow YAML SHOULD orchestrate reusable modules. It MUST NOT become a second
implementation of:

- Binary building.
- Native package assembly.
- Profile building.
- Manifest generation.
- Compatibility selection.
- Hashing or evidence generation.
- Package installation verification.
- Glow-up.
- Release diff policy.
- Channel distribution generation.
- Deployment verification.

Those operations MUST have reusable, checked-in entrypoints with explicit
inputs, outputs, and tests.

### 9.3 Immutable handoff

Every module handoff MUST identify inputs and outputs by immutable identity,
including version/revision and digest.

The source handoff MUST also carry the selected commit as one value. After the
complete local proof, the command creates or verifies a lightweight
`capsem-source-<40hex>` tag pointing exactly at it and dispatches the workflow
from that derived ref while passing the same required `source_commit` input.
Workflow entry and every reachable checkout MUST verify and use that commit;
an independently supplied ref, moving branch, implicit event SHA, or
title-only workflow correlation is insufficient.

Downstream jobs MUST verify those identities before use. A mutable artifact
name, mutable cache, branch-relative output, or unverified download MUST NOT be
accepted as release evidence.

CI SHOULD retain a content-addressed cache for large pulled artifacts. The
cache is a transport optimization, never authority: the current manifest is
still fetched on every run, selects the digest set, and is retained with the
resolved inputs. CI MUST download only missing or corrupt manifest-selected
blobs, SHOULD limit profile pulls to the runner architecture when the owning
test consumes one architecture, and MUST re-verify all selected blobs after
cache restoration.

Before immutable candidate URLs are publicly reachable, an installed test MAY
use a hermetic URL-only transport projection of the authoritative manifest.
The projection MUST reuse the exact package and profile bytes, MUST prove that
restoring the original URLs reproduces the authoritative manifest exactly, and
MUST NOT become a source manifest, public manifest, or alternate authority.

### 9.4 Read permission does not imply write permission

A lane may read complementary artifacts to prove compatibility. Reading them
does not allow that lane to regenerate, normalize, reserialize, copy over, or
rewrite their metadata.

The lane diff contract is evaluated over all persistent outputs, not merely
the workflow's intended upload list.

## 10. Lane invariants

Each row below is a named release contract. The proving test MUST fail if the
lane writes outside its allowed scope, even when the resulting bytes appear
valid.

| Lane | May write | Must never touch | Proving test |
|---|---|---|---|
| Binary release | Selected channel's packages, per-binary inventory, host SBOM, host attestations | Any profile data, any other channel | `test_binary_lane_gate` |
| Profile release | One channel+profile's images, config, evidence, revision, matching digests | Packages, binaries, other profiles, other channels | `test_profile_lane_gate` |
| Manifest validation | Channel definitions, bounds, membership | Artifact bytes of any kind | `test_release_lane_diff_policy` |
| Channel deploy | The generated public dist | Source manifests | `test_channel_deploy_contract` |
| Corporate authoring | The corporation's own manifest and profiles | Our channels, our binaries | `test_corporate_manifest_contract` |

### 10.1 Binary lane invariant

The binary lane:

- MAY create packages for the selected channel.
- MAY create or update the selected channel's per-binary inventory.
- MAY create or update host SBOM and host attestations.
- MAY read every selected channel profile and its assets.
- MUST NOT build profile images.
- MUST NOT modify profile configuration, evidence, revision, or digests.
- MUST NOT update another channel, including the other first-party binary
  channel.

`test_binary_lane_gate` MUST compare before/after release state and prove that
all profile-owned paths and all non-selected channel paths are byte-for-byte
unchanged.

### 10.2 Profile lane invariant

The profile lane:

- MUST select exactly one channel and one profile.
- MAY build all assets contained by that selected channel/profile.
- MAY update that channel/profile's configuration, evidence, revision, and
  matching digests.
- MAY read existing official binaries and packages for compatibility proof.
- MUST NOT build or rewrite packages or binaries.
- MUST NOT modify another profile in the selected channel.
- MUST NOT modify the same profile name in another channel.
- MUST NOT modify any other channel.

`test_profile_lane_gate` MUST use fixtures containing multiple channels and
multiple profiles, including the same profile name in two channels, and prove
that only the selected pair changes.

### 10.3 Manifest validation lane invariant

The manifest validation lane:

- MAY produce validated channel definitions, compatibility bounds, membership,
  and references through `capsem-admin`.
- MAY select staged or existing immutable artifacts by identity.
- MUST NOT create, edit, normalize, or republish artifact bytes.
- MUST reject references whose digests, bounds, ownership, or membership do
  not validate.

`test_release_lane_diff_policy` MUST prove both directions:

1. Every allowed manifest-only change is accepted.
2. Any artifact-byte change in a manifest-only run is rejected.

### 10.4 Channel deploy lane invariant

The channel deploy lane:

- MAY deploy only a complete generated public distribution.
- MUST verify the distribution before activation.
- MUST NOT read source manifests as an alternate deployment input.
- MUST NOT edit source manifests.
- MUST NOT rebuild packages, binaries, profiles, or assets.
- MUST NOT repair or fill missing generated output during deployment.

`test_channel_deploy_contract` MUST prove that deployment accepts a complete
generated distribution and rejects source manifests, partial distributions,
digest drift, and attempts to synthesize missing files.

### 10.5 Corporate authoring lane invariant

The corporate lane:

- MAY use `capsem-admin` to write only the corporation's manifest, channels,
  profiles, and profile-owned assets.
- MAY reference supported official Capsem binary artifacts read-only.
- MUST NOT build or overwrite official Capsem binaries.
- MUST NOT mutate first-party manifests, channels, or profiles.
- MUST enforce namespace and destination ownership before any write.

`test_corporate_manifest_contract` MUST prove exact pins, supported latest
selection, corporation-owned channel/profile authoring, rejection of official
binary writes, rejection of first-party channel writes, and rejection of any
authoring path that bypasses `capsem-admin`.

## 11. Reusable CI modules

The implementation MUST provide reusable modules for the following
capabilities. Workflow names are deliberately unspecified.

### 11.1 Resolve immutable release inputs

Inputs:

- Selected channel.
- Optional selected profile.
- Binary version or approved resolver policy.
- Manifest identity.

Outputs:

- Concrete immutable identifiers and digests.
- The compatibility cohort to test.

The resolver MUST fail before expensive work if any selection is ambiguous,
missing, mutable without resolution, or outside the requested ownership scope.
Normal workflow logs MAY report the resolution. No additional release-authority
file is required.

### 11.2 Build Capsem binaries

This module MUST:

- Build the requested platform and architecture matrix.
- Produce an immutable binary cohort.
- Record the binary version and selected source commit on every package row in
  the manifest; GitHub also records the source in the ordinary run log.
- Produce per-binary inventory inputs.
- Avoid reading profile source inputs except where package assembly needs
  immutable, already-selected profile references.

It MUST NOT build profile assets.

### 11.3 Assemble native packages

This module MUST:

- Assemble the exact candidate packages intended for publication.
- Include the exact binary cohort selected for the release.
- Produce package hashes and metadata.
- Produce the host SBOM and required attestations.
- Preserve referenced profile data rather than regenerating it.

### 11.4 Install and verify exact packages

This module MUST:

- Install the exact candidate package bytes.
- Exercise actual post-install behavior.
- Verify installed versions, binary inventory, service registration, and
  launchable product surfaces.
- Verify the installed manifest and update state.
- Fail if package installation only appears successful while the product is
  unusable.

Source-layout checks, archive expansion, or package-manager exit status alone
do not satisfy this module.

### 11.5 Build one channel/profile

Inputs MUST include:

- Exactly one channel.
- Exactly one profile.
- The declared profile revision from which `capsem-admin` derives the immutable
  publication identity.
- Required platform and architecture targets.

This module MUST:

- Build the profile's complete required assets.
- Generate profile configuration, evidence, revision, and matching digests.
- Record immutable output identities.
- Produce no package or binary output.
- Fail if output escapes the selected channel/profile namespace.

### 11.6 Generate and validate manifests

This module MUST invoke `capsem-admin` and MUST:

- Define or update channel membership.
- Select immutable binary and profile artifacts.
- Apply minimum and maximum compatibility constraints.
- Verify all references and digests.
- Reject missing or contradictory relationships.
- Generate the manifest output used by distribution assembly.
- Produce no artifact bytes.

### 11.7 Run binary/profile compatibility

This module MUST support:

- Candidate binary against existing profiles.
- Candidate profile against the channel's existing selected binary.
- Candidate binary against a staged candidate profile.
- Existing binary against existing profiles as the preserved baseline.

It MUST exercise real runtime behavior where the compatibility claim includes
runtime operation. Parser-only checks are insufficient.

### 11.8 Run glow-up

Glow-up MUST operate on an installed product and MUST exercise the state
transitions relevant to the release, including:

- Fresh installation from an exact package.
- Binary upgrade while preserving compatible profile/VM state.
- Profile refresh without unnecessary binary replacement.
- Channel transition where supported.
- Integrity failure with preservation of the previously working state.
- Service and command functionality after every transition.

The same reusable glow-up implementation MUST be used by nightly and stable
release orchestration. Workflows MUST NOT contain reduced channel-specific
copies.

### 11.9 Prove lane boundaries

This module MUST:

- Snapshot all lane-visible release state before execution.
- Snapshot it again after candidate construction.
- Classify every change by owner, channel, profile, and artifact type.
- Reject any write not explicitly allowed by the selected lane.

The named lane contract tests MUST exercise this behavior in isolated temporary
directories. Before/after hashes and classifications are test implementation
details. They MUST NOT become a published write-set, release-result file, or
second release authority.

### 11.10 Assemble generated public distribution

This module MUST:

- Combine validated manifest output with immutable artifact references.
- Preserve unchanged lane-owned data exactly.
- Produce a complete deployable distribution.
- Verify all internal references, identities, digests, and compatibility
  relationships.
- Produce deterministic output for identical inputs.

### 11.11 Deploy and verify channel

This module MUST:

- Accept only the generated public distribution.
- Serialize activation where concurrent writes could race.
- Preserve the prior public distribution until the new one is fully verified.
- Activate the new distribution atomically or with equivalent fail-safe
  behavior.
- Poll the public endpoint after activation.
- Verify that public content matches the candidate distribution.
- Report the activated immutable identities.

## 12. Release flows

### 12.1 Binary-only release

Use this flow when Capsem changes but no profile needs rebuilding.

1. Run `just release-binaries <channel> <source-commit>`.
2. Acquire the shared per-channel lock.
3. Resolve the selected channel's existing profile set and immutable assets.
4. Build the binary cohort.
5. Assemble exact native packages.
6. Produce per-binary inventory, host SBOM, and host attestations.
7. Test the candidate binary against every existing channel/profile that will
   remain published in that channel.
8. Install and verify every exact publishable native package on its required
   platform.
9. Run the applicable glow-up transitions using existing profile assets.
10. Prove with `test_binary_lane_gate` that no profile data and no other
    channel changed.
11. Generate the updated manifest through `capsem-admin`.
12. Run manifest validation and `test_release_lane_diff_policy`.
13. Assemble the generated public distribution.
14. Verify the candidate distribution.
15. Deploy through the reusable channel deploy lane.
16. Verify the public channel before reporting success.

The binary lane MUST NOT invoke profile, kernel, initrd, rootfs, or image
builders. A failure against pulled assets is a binary compatibility failure.

### 12.2 Profile-only release

Use this flow when one profile changes and existing channel binaries remain
compatible.

1. Run `just release-profile <channel> <profile> <source-commit>`.
2. Acquire the shared per-channel lock.
3. Pull the channel's currently selected released binary and package.
4. Build the selected profile and all assets contained by it.
5. Generate its configuration, evidence, revision, and matching digests.
6. Test the candidate profile against the pulled existing binary.
7. Boot and exercise the candidate profile through the real Capsem runtime on
   required host architectures.
8. Prove with `test_profile_lane_gate` that packages, binaries, other profiles,
   and other channels are unchanged.
9. Generate the updated manifest through `capsem-admin`.
10. Validate membership, bounds, artifact identities, and digests.
11. Assemble the generated public distribution.
12. Deploy through the reusable channel deploy lane.
13. Verify the public channel and automatic profile refresh behavior.
14. Report success to the caller only after the exact correlated workflow run
    has succeeded.

This flow MUST NOT invoke Rust release-binary or native-package construction.

### 12.3 Composed profile-then-binary release

Use this flow when:

- A new profile requires a Capsem capability not present in the channel's
  existing binary.
- A changed profile legitimately raises its minimum binary requirement.
- A compatible channel state requires both profile and binary movement.

This is an ordered composition of the two public commands, not a new lane,
command, or expanded permission set:

```text
just release-profile <channel> <profile> <source-commit>
just release-binaries <channel> <source-commit>
```

#### Phase A: build the profile candidate

1. Select exactly one channel/profile.
2. Build its complete asset set.
3. Produce configuration, evidence, revision, and matching digests.
4. Run all profile self-consistency, integrity, architecture, and boot proofs
   possible with the declared compatibility relation.
5. If the pulled existing binary is intentionally too old, express the unmet
   dependency through the profile's minimum binary bound.
6. Keep the profile candidate staged and inactive.
7. Prove that only the selected channel/profile candidate state was written.

The profile candidate MUST NOT be activated while the selected public channel
still points to an incompatible binary.

#### Phase B: build the binary candidate

1. Reacquire the same channel lock and consume every already-published
   immutable staged profile candidate as a read-only compatibility input.
2. Build the binary and exact native packages.
3. Test the candidate binary against:
   - Every staged candidate profile.
   - Every unchanged existing profile in the selected channel.
   - Any existing profile revision that must remain valid during update.
4. Install and verify the exact packages.
5. Run the composed glow-up path.
6. Prove that the binary phase wrote no profile data.

#### Phase C: validate the combined channel state

The compatibility matrix MUST explicitly cover:

| Binary | Profile | Expected result |
|---|---|---|
| Existing binary | Existing profiles | Remains valid until activation |
| Candidate binary | Existing unchanged profiles | Must pass |
| Candidate binary | Candidate profile | Must pass |
| Existing binary | Candidate profile requiring the new binary | Must be rejected by compatibility bounds |

The rejected old-binary/new-profile pair is not a test failure when the
manifest intentionally excludes it. Failure to reject that pair is a manifest
validation failure.

#### Phase D: generate and activate

1. Use `capsem-admin` to validate the source manifest state already authored by
   the profile command plus the binary selection authored by the binary lane.
2. Validate all bounds, membership, identities, and digests.
3. Run every lane diff contract.
4. Assemble one complete generated public distribution.
5. Activate it only after all binary, profile, manifest, package, glow-up, and
   deployment prerequisites pass.

There MUST NOT be an intermediate public state that exposes only half of the
required pair.

### 12.4 Manifest-only release

Use this flow for changes limited to channel definitions, bounds, or membership
that reference existing immutable artifacts.

A manifest-only release MAY:

- Change channel membership.
- Change selected immutable versions.
- Tighten or widen compatibility bounds when evidence supports the claim.
- Add or remove references to already-built channel/profile instances.

It MUST NOT:

- Change any profile definition.
- Change profile or package bytes.
- Recalculate digests to conceal changed bytes.
- Treat a profile content change as metadata-only.

If profile content changes, the operation is a profile release. If binary or
package content changes, the operation is a binary release.

### 12.5 Channel deploy

Both binary and profile flows end by invoking the same reusable deploy lane.

The deploy lane:

1. Accepts the generated public distribution.
2. Verifies its identity and completeness.
3. Verifies the lane diff evidence.
4. Deploys without rebuilding or editing source state.
5. Verifies the public result.
6. Leaves the previous public state available if activation fails.

### 12.6 Corporate release

The corporate flow is:

1. The corporation uses `capsem-admin` to define its manifest and channels.
2. It selects exact or policy-resolved official Capsem binary versions.
3. It defines the profiles belonging to each corporate channel.
4. It builds its channel/profile instances and contained assets through
   `capsem-admin`.
5. It validates compatibility, integrity, and ownership.
6. It generates its corporate manifest and distribution.
7. Installed corporate Capsem instances poll that manifest.

The corporate release path MUST reuse the same profile, manifest,
compatibility, integrity, and diff-policy modules as first-party release paths.
It MUST add ownership enforcement; it MUST NOT replace those modules with a
less strict corporate implementation.

## 13. Scheduling and triggers

### 13.1 Push and pull-request CI

Push and pull-request CI provide fast correctness feedback. They MUST NOT
publish a nightly binary on every push.

Their precise test partitioning is an implementation decision, but failures in
required checks MUST block merge or candidate selection according to project
policy.

### 13.2 Nightly orthogonal schedule

Nightly binary and selected profile/asset rebuilds SHOULD run once daily.

Each run MUST:

- Freeze `${{ github.sha }}` from the current releasable `main` state selected
  by the scheduler, and use that same full SHA for every checkout and lane even
  if `main` advances during the run.
- Target only the nightly channel.
- Invoke `just release-profile nightly <profile> <source-commit>` separately
  for every selected profile, then invoke
  `just release-binaries nightly <source-commit>`; it MUST NOT dispatch a
  downstream workflow or combine artifact ownership itself.
- Wait for each exact correlated profile run before starting another
  same-channel command. Profile commands MAY be ordered serially while each
  profile workflow keeps its independent artifact ownership.
- Run the binary command after all scheduled profile commands have terminated,
  even when one profile lane failed, so one orthogonal family cannot suppress
  the other family's daily rebuild.
- Rebuild nightly profile assets rather than resolving a prior workflow's build
  artifacts. Stable retry MAY reuse one exact verified prior artifact cohort.
- Rebuild and test binary packages against the manifest-selected nightly
  profiles every day.
- Publish and activate only when the current version introduces a new immutable
  release identity. When that identity already exists, run the same package,
  native-install, functional, Winterfell, IronBank, and glow-up proof with
  publication disabled; signed/notarized bytes MUST NOT overwrite an existing
  tag.

The scheduler has its own non-cancelling lock to prevent overlapping daily
orchestrators. The downstream binary and profile workflows retain the shared
`capsem-release-nightly` transaction lock from manifest resolution through
deployment. The daily schedule rebuilds current `main` without converting
every push into a publication.

### 13.3 Stable binary trigger

Stable binary publication MUST be explicitly and manually initiated.

The trigger MUST select:

- One prepared binary version.
- The stable channel.
- One full source commit already on `main`.
- Any required release metadata.

Stable MUST use the same reusable binary, package, compatibility, glow-up,
manifest, distribution, and deploy modules as nightly. It MAY have stricter
approval requirements, but MUST NOT use duplicated build or test logic.

### 13.4 Profile trigger

A profile release MUST be explicitly targeted with:

- One channel.
- One profile.
- One full source commit already on `main`.

The trigger MUST reject:

- Missing channel or profile.
- Wildcard profile selection.
- “All channels” as an implicit convenience.
- A profile that is not a member of the selected channel unless the same
  `capsem-admin` operation explicitly and validly adds that membership.
- Any attempt to mutate the same profile name in another channel.

### 13.5 Profile-then-binary trigger

There is no composed-release trigger. The administrator invokes the two public
commands in order. The staged profile remains inert until the following binary
release validates the completed graph and performs one final activation.

## 14. Automatic client update behavior

### 14.1 Polling

Capsem regularly polls its configured manifest. Update behavior is automatic;
the release design MUST NOT depend on users manually downloading or assembling
components.

The polling interval and backoff policy are implementation choices, but polling
MUST:

- Fetch the selected manifest safely.
- Verify manifest identity and integrity before trusting references.
- Resolve the current channel.
- Compare installed and selected binary/profile state.
- Verify compatibility before mutation.
- Fetch only required changed artifacts.
- Preserve the working installed state on any failure.

### 14.2 Binary-only update

When only the selected binary changes:

- Capsem updates the verified package/binary state.
- Unchanged profile assets MUST remain unchanged.
- Compatible VM/profile state MUST not be invalidated merely because host code
  was fixed.
- Post-update checks MUST prove the package, service, and existing profiles
  remain functional.

### 14.3 Profile-only update

When one selected channel/profile changes:

- Capsem fetches and verifies that profile's new complete asset set.
- Other profiles remain unchanged.
- The same profile name in another channel remains unchanged.
- The Capsem binary remains unchanged when it satisfies the new compatibility
  constraints.
- Any necessary profile-specific VM replacement or migration is confined to
  that channel/profile.

### 14.4 Combined update

When a manifest advances both binary and profile:

- Capsem MUST compute a safe ordered update plan.
- It MUST verify all required bytes before discarding the working state.
- It MUST never boot the new profile with a binary that the manifest declares
  incompatible.
- It MUST never finalize the new binary while leaving an incompatible selected
  profile.
- Failure MUST preserve or restore the last complete compatible state.

### 14.5 Fail-closed integrity

Digest mismatch, missing evidence, incompatible versions, incomplete profile
assets, invalid membership, or a partial package cohort MUST prevent mutation.

The updater MUST NOT:

- Fall back silently to another channel.
- Substitute another profile.
- accept mutable bytes under a previously verified identity.
- Treat missing integrity data as an empty or unchanged result.
- Partially apply an update and report success.

## 15. Package and glow-up proof

### 15.1 Exact artifact requirement

The bytes tested MUST be the bytes intended for publication.

Each supported native package MUST be:

- Built as a release candidate.
- Identified by immutable version and digest.
- Installed using the real platform installation path.
- Verified after installation.
- Exercised through real public product commands and services.

### 15.2 Required installed-product assertions

At minimum, package proof MUST verify:

- Native package metadata and receipt.
- Exact installed Capsem version.
- Complete installed binary cohort.
- Binary hashes or inventory consistency.
- Required service registration and startup.
- A functional Capsem command.
- Manifest installation and polling state.
- Existing profile readiness.
- Real profile boot or guest command where the platform provides the required
  virtualization capability.

### 15.3 Glow-up transitions

The glow-up suite MUST cover the state transitions necessary to prove package
and updater behavior, not independent fresh installations pretending to be an
upgrade.

Required scenarios include:

- Existing supported binary plus existing profiles to candidate binary plus
  unchanged profiles.
- Existing profile revision to candidate profile revision with unchanged
  compatible binary.
- Combined binary/profile movement where required.
- Supported channel switching and return switching.
- Rejection of tampered manifest or artifact bytes.
- Preservation of the prior installed state after rejection.
- Healthy service and functional command after every successful transition.

An empty first-party bootstrap source is not a successful installed state and
MUST NOT be inserted into this transition list merely to manufacture an upgrade.
The first activated profile is proved as a fresh candidate pairing; later
profile releases retain the required existing-profile to candidate-profile
transition.

## 16. Integrity, provenance, and evidence

Every persistent release artifact MUST have:

- An immutable identity.
- A cryptographic digest.
- A byte size where relevant.
- An owning lane.
- An owning channel and, for profile data, profile.
- Compatibility metadata where relevant.

Manifest references MUST match the artifact evidence exactly.

The system MUST distinguish:

- Host package and binary evidence.
- Profile and VM asset evidence.
- Manifest validation evidence.
- Deployment evidence.

One lane MUST NOT regenerate another lane's evidence. Preservation is by exact
reference or exact unchanged bytes, not by lossy conversion.

The manifest is the release authority. Every newly authored package row MUST
record its family's exact `source_commit`; every newly authored selected
profile document MUST record its family's exact `source_commit`. These fields
are optional on read so legacy and gradual mixed graphs remain readable, but a
present value is exactly 40 lowercase hexadecimal characters and explicit null
is invalid. Source commit MUST NOT appear at graph top level or per-binary:
those placements imply ownership wider or narrower than the publishing family.
Status-only updates preserve existing provenance, and one lane preserves the
other family's fields unchanged.

Existing host SBOMs, profile OBOMs, attestations, structured gate run logs, and
GitHub workflow logs supply the remaining evidence. The system MUST NOT add a
parallel state, history, transaction, recovery, or result document. Run id is
attempt identity and remains distinct from source commit.

## 17. Publication and transaction safety

### 17.1 Candidate bytes are inert

Immutable packages, profiles, or assets MAY be uploaded before activation.
They MUST remain inert until a validated generated public distribution
references them.

### 17.2 No partial activation

A channel MUST move from one complete valid graph to another complete valid
graph.

The following states MUST never be observable as the selected public state:

- New binary with missing required package architecture.
- New binary with profiles it cannot run.
- New profile with an incompatible selected binary.
- Manifest references to assets not yet available.
- Mixed digests from two candidate runs.
- Updated membership with stale profile data.

### 17.3 Concurrency

Candidate construction MAY run in parallel when outputs and caches are
isolated.

Activation affecting the same channel MUST be serialized. Two jobs MUST NOT
race to merge independently generated partial channel state.

Builds for different channel/profile pairs MAY run concurrently, but their
eventual channel activation still requires a fresh validated distribution
assembled from the intended complete state.

### 17.4 Failure behavior

Before activation, failure leaves the public channel unchanged.

During activation, failure MUST either:

- Leave the previous distribution active, or
- Roll back automatically to the previously verified distribution.

The release MUST not report success until public verification confirms the
expected manifest and artifact identities.

### 17.5 Forward-only immutable history

Published immutable artifact identities MUST NOT be overwritten or reused.

A failed candidate receives a new forward candidate after correction. Audit
history must remain sufficient to explain which bytes were built, tested,
staged, activated, rejected, or superseded.

## 18. Contract test requirements

### 18.1 `test_binary_lane_gate`

This test MUST:

- Construct at least two channels.
- Place multiple profiles in the selected channel.
- Place at least one profile in another channel.
- Run a binary-lane fixture for one selected channel.
- Prove packages, per-binary inventory, host SBOM, and host attestations may
  change only in that channel.
- Prove all profile config, images, evidence, revisions, and digests remain
  byte-for-byte unchanged.
- Prove the other channel remains byte-for-byte unchanged.
- Fail on profile rebuilding, profile metadata normalization, or cross-channel
  package writes.

### 18.2 `test_profile_lane_gate`

This test MUST:

- Construct at least two channels and multiple profiles.
- Include the same profile name in two channels.
- Select exactly one channel/profile.
- Prove that selected profile images, config, evidence, revision, and matching
  digests may change.
- Prove packages and binaries remain unchanged.
- Prove sibling profiles remain unchanged.
- Prove the same profile name in the other channel remains unchanged.
- Fail on wildcard or implicit multi-profile output.

### 18.3 `test_release_lane_diff_policy`

This test MUST:

- Classify changes by lane owner, channel, profile, and artifact type.
- Accept every explicitly allowed write.
- Reject every forbidden write.
- Prove manifest validation can change channel definitions, bounds, and
  membership without changing artifact bytes.
- Reject digest changes that do not match selected immutable artifacts.
- Reject unclassified output.
- Reject deletion or normalization of another lane's evidence.

### 18.4 `test_channel_deploy_contract`

This test MUST:

- Accept a complete generated public distribution.
- Verify deployment consumes that distribution unchanged.
- Reject a source manifest as deployment input.
- Reject partial generated output.
- Reject mismatched digests and missing references.
- Reject deployment-time artifact generation or source-manifest editing.
- Prove failed deployment preserves the prior public state.

### 18.5 `test_corporate_manifest_contract`

This test MUST:

- Author the corporation's manifest and profiles through `capsem-admin`.
- Prove exact official binary pins.
- Prove an allowed latest policy resolves to an immutable official binary.
- Prove corporation-owned channel/profile independence.
- Reject attempts to write official binaries or packages.
- Reject attempts to write first-party channels or profiles.
- Reject namespace escape.
- Reject hand-written or alternate authoring paths.
- Prove integrity and compatibility validation uses the same production
  modules as first-party authoring.

## 19. End-to-end acceptance scenarios

The release architecture is not complete until all scenarios below pass.

### Scenario A: Capsem-only bug fix

Given:

- Stable and nightly each contain existing profiles.
- The profile bytes are known and immutable.
- A Capsem code fix changes no profile requirements.

When:

- The binary lane releases the fix to nightly.

Then:

- Only nightly packages and binary-owned evidence change.
- Nightly profiles are not rebuilt.
- Stable is unchanged.
- The candidate package installs successfully.
- Existing nightly profiles boot and function.
- Polling Capsem installations update automatically.
- Compatible VM/profile state is preserved.

### Scenario B: Stable manual binary release

Given a qualified exact binary candidate, when an administrator explicitly
starts stable publication, then stable uses the shared binary release modules,
passes exact-package and glow-up proof, and updates only stable binary-owned
state.

No push or nightly schedule may implicitly trigger this stable activation.

### Scenario C: One nightly-only experimental profile

Given:

- `experimental` belongs to nightly.
- `experimental` does not belong to stable.

When:

- `nightly/experimental` is released.

Then:

- Its complete assets are rebuilt.
- The currently selected nightly binary is pulled and tested with it.
- No stable data changes.
- No stable membership is invented.

### Scenario D: Same profile name in two channels

Given `default` exists in both stable and nightly, when `nightly/default`
changes, then `stable/default` remains byte-for-byte unchanged and retains its
own revision and digests.

### Scenario E: Profile-only compatible update

Given a profile change remains compatible with the channel's currently
selected binary, when that channel/profile is released, then only its
profile-owned outputs and manifest references change. No package is rebuilt.
If this is the channel's first profile, the empty bootstrap source remains
non-public and the candidate passes complete fresh-install, Doctor, Winterfell,
rejection, and preservation proof. Otherwise the installed glow-up also proves
the exact public-before profile to candidate-profile transition.

### Scenario F: Profile requires a new binary

Given a new profile revision requires a newer Capsem capability, when the
composed release runs, then:

- The profile is built first and remains staged.
- The binary is built second against the staged profile.
- The candidate binary also passes unchanged profiles.
- The manifest excludes the incompatible old-binary/new-profile pair.
- One complete compatible graph is activated.
- No intermediate incompatible graph becomes public.

### Scenario G: Corporate exact pin

Given a corporation authors its manifest through `capsem-admin` and pins an
official Capsem version, then its profiles build and validate against that
version without building a Capsem binary or changing any first-party channel.

### Scenario H: Corporate latest policy

Given a corporation selects an allowed latest policy, then `capsem-admin`
resolves it to an immutable official binary identity, records the resolution,
validates compatibility, and produces a reproducible manifest.

### Scenario I: Tampered update

Given a working installed state and a polled manifest or artifact with invalid
integrity metadata, the update fails closed, no partial state becomes active,
and the previously working binary/profile pair remains usable.

### Scenario J: Failed deployment

Given a fully qualified candidate but a failed public deployment or
post-deployment verification, the prior public distribution remains active and
the release is reported as failed.

## 20. Required release evidence

The manifest, existing SBOM/OBOM and attestations, and ordinary GitHub workflow
logs MUST together answer:

- Which lane ran?
- Which exact committed source did that lane qualify and build?
- Which channel was selected?
- Which profile and derived publication identity were selected, if any?
- Which immutable existing artifacts were consumed?
- Which candidate artifacts were produced?
- Which compatibility cohort was tested?
- Which exact packages were installed?
- Which glow-up transitions ran?
- Which named contract tests passed?
- Which manifest was generated through `capsem-admin`?
- Which generated public distribution was deployed?
- Which public identities and digests were verified after deployment?
- Did compatibility prevent public activation?

Missing evidence for a required claim is a failed release, not a warning.
No additional evidence ledger or result document is introduced.

## 21. Implementation acceptance checklist

An implementation conforming to this specification MUST demonstrate:

- [ ] Local `just test` constructs and validates the complete pipeline.
- [ ] Nightly binary and selected profile/asset rebuilds are scheduled daily
      through separate public commands rather than per push.
- [ ] Existing nightly identities are rebuilt and tested without overwriting
      immutable publications.
- [ ] Stable binary and profile publication is manual.
- [ ] `just test <source-commit>` requires one canonical full commit already
      on local `main`, qualifies its detached full-SHA prefix once, reuses or
      structurally resumes its journal, and remains valid while the outer
      checkout advances.
- [ ] Both release commands revalidate the complete archived journal before
      publishing or dispatching and never repeat the local candidate.
- [ ] Binary release accepts exactly one channel and one source commit.
- [ ] Profile release accepts exactly one channel, profile, and source commit and derives its
      immutable publication identity.
- [ ] Every release workflow checkout and correlated run uses that exact
      source commit and derived immutable transport ref.
- [ ] Newly authored package rows and selected profile documents record their
      family-owned source commit without a graph-wide provenance field.
- [ ] A profile can exist in zero, one, or multiple channels.
- [ ] Same-named profiles in different channels remain independent.
- [ ] Profile changes rebuild the selected profile's complete asset set.
- [ ] Binary releases test against existing profiles without rebuilding them.
- [ ] Profile releases test against the existing selected binary without
      rebuilding them.
- [ ] A composed profile-then-binary flow exists.
- [ ] The composed flow has one final activation and no incompatible
      intermediate public state.
- [ ] All first-party authoring goes through `capsem-admin`.
- [ ] All corporate authoring goes through `capsem-admin`.
- [ ] Corporations cannot build or mutate official Capsem binaries.
- [ ] CI build, package, profile, manifest, compatibility, glow-up, diff,
      distribution, and deploy logic is reusable rather than duplicated.
- [ ] Exact publishable native packages are installed and functionally tested.
- [ ] Capsem automatically polls and applies verified manifest updates.
- [ ] Unchanged profiles and VM state survive compatible binary updates.
- [ ] Unchanged binaries survive compatible profile updates.
- [ ] Every update fails closed on integrity or compatibility failure.
- [ ] `test_binary_lane_gate` passes.
- [ ] `test_profile_lane_gate` passes.
- [ ] `test_release_lane_diff_policy` passes.
- [ ] `test_channel_deploy_contract` passes.
- [ ] `test_corporate_manifest_contract` passes.
- [ ] Public deployment is verified before success is reported.
- [ ] Failure preserves the last complete working public and installed states.

## 22. Decisions intentionally left to implementation

This specification does not choose:

- Workflow filenames.
- The exact binary version syntax.
- The storage provider for immutable packages and profile assets.
- The CI vendor.
- The manifest polling interval and backoff.
- The UI used to trigger manual stable or profile releases.
- The internal language used for reusable CI modules.
- The retention duration for staged or superseded candidates.

Those choices may vary without changing the architecture, provided every
normative invariant and contract test in this document remains satisfied.

## 23. Summary of the governing contract

Capsem binaries and profiles are independently releasable but jointly tested.
A manifest authored through `capsem-admin` defines channels, each channel's
binary policy, and the profiles belonging to that channel. Profiles contain
their assets. The same profile name may exist independently in multiple
channels, and a profile need not exist in every channel.

Local `just test` rebuilds and verifies the complete world. CI is selective:
the binary lane builds binaries and packages and tests them against existing
profiles; the profile lane rebuilds exactly one channel/profile and tests it
against the channel's existing selected binary. When both must move, CI builds
the profile first, builds the binary against it second, and activates one
compatible manifest state only after both independent lanes pass.

All release orchestration reuses the same checked-in modules. Lane write scopes
are enforced by named contract tests. Exact packages are installed and proven
through glow-up. Generated distributions deploy atomically and fail closed.
Capsem polls manifests and updates automatically without requiring users to
manually download or coordinate components.
