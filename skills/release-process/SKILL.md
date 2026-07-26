---
name: release-process
description: Capsem release process, orthogonal binary/profile CI, Apple code signing, notarization, channel deployment, and post-release verification.
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

## One command owns the complete release

Capsem has exactly two release-facing Just commands:

```bash
just release-binaries <channel>
just release-profile <channel> <profile>
```

These are the sole release entrypoints for humans and checked-in automation.
Do not ask an operator to run a preparation command or a separate `just test`
first. Each release command itself has this non-negotiable order:

```text
just release-binaries <channel>
  1. just test
  2. only after success: run the binary release script and dispatch binary CI

just release-profile <channel> <profile>
  1. just test
  2. only after success: invoke capsem-admin release for that channel/profile
```

`just test` must be the first consequential command. If it fails, the release
command must stop before stamping versions, changing tracked files, committing,
tagging, pushing, authoring a shared manifest, or dispatching a workflow. Test
this fail-stop behavior by executing the public recipes with fake downstream
commands; inspecting recipe text alone is insufficient.

Do not introduce a skip flag, release-only reduced gate, preparation recipe,
environment-variable bypass, or direct checked-in caller of:

- `scripts/release-binaries.py`;
- `capsem-admin release` for a first-party public profile;
- `release.yaml` or `release-assets.yaml`.

Daily nightly automation calls `just release-binaries nightly`; it does not
dispatch the workflow directly. The supported first-party profile path calls
`just release-profile`. Direct GitHub UI dispatch is not the documented or
tested release path.

Each command owns one artifact family. There is no combined release command.
The commands may run sequentially when a profile requires new code, but neither
may rebuild the other command's artifact family.

`config/public-surface.toml` locks this command surface. Treat any change as an
explicit product/API decision.

## Local proof and release-CI composition

`just test` is the complete local CI-equivalent proof, not a smaller developer
smoke test. It rebuilds every package and every checked-in profile, then runs
all five checked-in modules:

- `_test-static`
- `_test-artifacts`
- `_test-functional`
- `_test-glowup`
- `_test-release-contracts`

Every test, scanner, contract, build validation, and tool dependency required
by release CI must be reachable from this command. A gate that exists only as
inline workflow YAML is a parity defect until it is extracted into a
checked-in module called by `just test`. Each module must own its prerequisites
and must also be executable independently in a clean local environment. Never
rely on a package installed incidentally by an earlier workflow job or by a
developer machine.

The cheap failures run before VM and artifact work. They include formatting,
lint, Rust clippy, Python checks, JavaScript/frontend checks, action/workflow
validation, source contracts, and vulnerable-dependency audits for every
locked ecosystem. The complete proof still includes all expensive gates:
artifact validation and boot, every VM suite, Winterfell, MCP lifecycle,
IronBank, injection, integration, benchmarks, full `capsem-doctor`, native
package installation, and glow-up transitions. None is advisory.

Release automation uses the same public command and therefore receives the
same complete `just test` gate before dispatch. The dispatched release
workflows then save construction time, never test quality:

- the binary lane builds packages only and resolves every selected-channel
  profile by manifest-recorded digest;
- the profile lane builds exactly one channel/profile and resolves the
  selected channel's current package by manifest-recorded digest;
- both lanes stage those exact resolved artifacts into the same test modules
  used locally;
- source-built substitutes must not replace the resolved complementary family.

This is one test architecture with two artifact-preparation modes, not a local
test path and a forked CI test path. The test modules must not silently choose
different assertions based on ambient release environment variables. Artifact
preparation may differ—local builds both families, a release lane downloads the
unchanged family—but the resulting manifest-addressed bundle enters the same
module implementations.

Before public activation, the resulting pairing must pass manifest/artifact
integrity, every VM suite, Winterfell and MCP lifecycle, IronBank, injection,
integration, benchmarks, full `capsem-doctor`, native install, and update
glow-up. A staged incompatible profile may run only static, self-consistency,
integrity, isolation, and boot gates; the following binary lane must run the
complete functional and glow-up proof before activation.

The local gate records `HEAD` and a digest of all tracked and untracked
non-ignored source bytes. It supports ordinary uncommitted development and
fails if the source state changes while tests run.

Before dispatching a real release, run the actual public release command, not
`just test` followed by a hand-written dispatch. Its embedded `just test` is
the local proof and its remaining steps are the only supported bridge into CI.
Do not dispatch CI until that embedded local proof completes successfully.

## Shared per-channel serialization

Both production entry workflows use exactly:

```yaml
concurrency:
  group: capsem-release-${{ inputs.channel }}
  cancel-in-progress: false
```

The workflow acquires the lock before reading the source manifest and holds it
through artifact resolution, tests, source-manifest mutation, generated
distribution assembly, and production deployment.

Consequences:

- binary and profile release work for one channel cannot overlap;
- two profile releases for one channel cannot overlap;
- queued work re-reads the manifest only after acquiring the lock;
- stable and nightly may proceed concurrently;
- preview deployment cannot mutate production source manifests;
- `release-channel.yaml` may deploy production only for a serialized parent
  binary or profile workflow.

`release-channel-staging.yaml` is the preview-only proof of the reusable
deployer. It renders a deterministic generated distribution and deploys a
non-production branch without invoking VM asset builds or host package builds.

The selected channel source manifest is the sole mutable release authority. Do
not add a release result file, pending ledger, last-known-good graph, manual
diff approval record, or parallel authoring path.

## Profile release

`just release-profile nightly code` invokes:

```bash
capsem-admin release --channel nightly --profile code
```

The locked profile workflow:

1. reads the latest nightly source manifest;
2. resolves and verifies its current package;
3. builds only the `nightly + code` config, images, inventory, OBOM, evidence,
   and architecture cohort;
4. creates an immutable identity containing channel and profile identity;
5. validates digests, bootability, and the unchanged package pairing;
6. mutates only the selected profile entry;
7. deploys immediately when the public package satisfies the profile's
   declared minimum Capsem version.

If the public package is too old, publish the immutable profile artifacts and
persist the staged source-manifest state, but do not deploy that incompatible
pairing. Other profiles, channels, packages, and binaries remain untouched.

All corporate manifest and profile authoring also goes through `capsem-admin`.
A corporation owns its manifest and profile definitions, may use the latest
compatible Capsem package or pin a compatible version, and never builds or
mutates Capsem-owned binaries or public channels.

## Binary release

`just release-binaries nightly` invokes the checked-in, adversarially tested
binary release script. The locked binary workflow:

1. reads the latest nightly source manifest;
2. resolves every referenced profile by recorded digest, including compatible
   staged profiles;
3. builds only candidate packages, per-binary inventory, host SBOM, and
   existing attestation evidence;
4. runs the complete functional suite for every resulting channel profile;
5. installs the exact native packages and runs binary-update plus
   profile-then-binary glow-up;
6. mutates only package, per-binary inventory, host SBOM, and existing
   attestation fields;
7. assembles and deploys the completed channel only after every gate passes.

The workflow must never invoke a profile/image builder.

Daily nightly automation calls this same binary command path and queues behind
other nightly release work. It does not publish on every push. Stable uses the
same command explicitly and the same quality gates.

## Dependent profile then binary release

When a profile requires new Capsem code:

1. run `just release-profile <channel> <profile>`;
2. publish the immutable assets once and withhold the incompatible public
   channel;
3. run `just release-binaries <channel>`;
4. resolve the already-built staged profile by digest;
5. run the full functional, native install, and glow-up proof over the
   completed pairing;
6. activate the channel only after success.

Neither artifact family is rebuilt twice.

## Evidence and integrity

The manifest defines channel membership, profiles, compatibility bounds,
packages, binaries, and integrity digests. SBOM, OBOM, existing attestations,
and GitHub workflow logs are the release evidence. Do not add another
provenance or approval document.

Profiles belong to channels. A profile may appear in several channels, one
channel, or no public channel; each channel/profile publication is independent.
Every immutable config, image, evidence, and revision path must include enough
channel/profile identity to prevent stable and nightly from aliasing bytes.

Public graph rules:

- release graphs and local asset manifests are generated through
  `capsem-admin manifest generate`;
- packages are delivery containers;
- per-binary inventory stays under its owning package;
- profiles own config, images, inventory, OBOM, evidence, and their minimum
  compatible Capsem version;
- mutable channel pointers use
  `Cache-Control: no-cache, must-revalidate`;
- immutable artifacts use
  `Cache-Control: public, max-age=31536000, immutable`;
- every fetched artifact is verified by recorded digest before use.

Read `references/release-graph.md` before changing graph generation or channel
deployment.

## Native installation and platform gates

Native installation is a functional outcome, not a file-existence check:

- macOS CI builds the publishable `.pkg`, signs, notarizes, staples,
  Gatekeeper-checks, installs that exact package, verifies the full binary
  cohort and service, and preserves the local Apple VZ proof boundary;
- Linux CI builds every required `.deb`, installs each host-native exact
  package, verifies package metadata, binaries, service, and command behavior,
  and runs the mandatory guest shell where KVM is available;
- publication depends on both platform rails;
- skipped, optional, source-layout-only, or inspect-only checks do not count;
- `scripts/verify-installed-release.py` verifies the exact installed manifest,
  metadata sidecar, profile readiness, package version, and update state;
- the stateful glow-up proves binary-only, profile-only,
  profile-then-binary, channel switching, tamper rejection, and preservation
  of the previous working state, with Winterfell and full doctor after
  transitions.

GitHub-hosted macOS cannot repeat nested Apple Virtualization.framework guest
boot. Local Apple Silicon `just test` owns that VZ proof. Hosted macOS owns
signing, notarization, stapling, installation, and structural verification of
the final publishable package. Neither substitutes for the other.

The installed source of truth remains the exact verified
`assets/manifest.json`, byte-for-byte. Installation and update code must not
rewrite it into a reduced runtime schema. The only metadata sidecar is
`assets/manifest-metadata.json` with schema
`capsem.manifest_metadata.v1`; do not create a separate origin file. Runtime
adapters may derive an in-memory boot view. `GET /system/status` returns that
manifest, metadata, readiness, corporate state, and update comparison. CLI and
UI consume the same status contract; the UI must not synthesize publication
state.

Read `references/apple-signing.md` when touching signing, notarization,
certificates, Tauri keys, or Apple agreements. Read
`references/post-release-verification.md` after any public deployment.

## Failure and retry discipline

- A red gate stops publication.
- Fix forward with a normal commit; never move a published tag or rewrite
  public release history.
- Do not blindly rerun unchanged work when the failure is deterministic.
- Preserve the previous public channel and installed working pair on any
  artifact, compatibility, tamper, test, or deployment failure.
- Treat disk, runtime, and runner capacity as tested release resources.
- Keep expensive artifact staging hardlink-first on the same-filesystem, with
  a tested cross-filesystem copy fallback and constrained-disk regression.
- Keep the clean-environment bootstrap proof before expensive work, while
  retaining the full installer E2E later.

Read `references/ci-invariants.md` before editing release workflows. It carries
the platform, toolchain, scanner, disk, Docker, package, and runner lessons
learned from prior failures.

## Release-channel Cloudflare prerequisites

Before running a live binary or profile channel deploy, verify the Cloudflare
Pages project serving `release.capsem.org`, its `release.capsem.org` custom
domain, and both `CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN`.
After deployment, run `scripts/check-release-site-contract.py`; it validates
BLAKE3/SHA-256 content, graph agreement, attestation references, and cache
headers rather than only checking that files exist.

## Documentation, changelog, and versions

Documentation and marketing deploy independently from binary/profile release
rails. Their builds remain mandatory source gates.

Keep user-visible changes under `## [Unreleased]` in `CHANGELOG.md`. Historical
entries describe past behavior and are not normative release instructions.

Binary and profile versions are orthogonal:

- binary: the Capsem package/application version;
- profile: the immutable channel/profile publication identity derived and
  authored by `capsem-admin`.

Do not infer that a profile change requires a binary rebuild, or that a binary
change requires rebuilding any profile.

## Commit discipline

1. Include the appropriate `CHANGELOG.md` entry for user-visible changes.
2. Stage files explicitly.
3. Use conventional commit subjects.
4. Never stage private release material, certificates, keys, tokens, or
   local-only credentials.
