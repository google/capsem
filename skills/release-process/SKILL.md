---
name: release-process
description: Capsem's Python-owned release process, orthogonal binary/profile CI, Apple code signing, notarization, channel deployment, diagnostic-continuation boundary, and post-release verification. Use for release commands, release failures, manifests, publication workflows, or any change that could affect what qualifies or ships.
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

## Python owns release orchestration

The Justfile is only the public command and argv boundary: each release recipe
dispatches its arguments once to the matching `uv run capsem-gate` subcommand.

Python under `src/capsem/gate/` owns the release graph:

- `release.py` declares commands/publication edges; `candidateplan.py` composes
  the complete test fragments.
- `qualification.py` parses one legal local/binary/profile state.
- `command.py` validates, inspects, locks, holds, records, and executes.
- Actions perform work, resources own lifecycle/evidence, and
  `config/gate.toml` owns values.

The release command does **not** launch `just test`, Just, or another
`capsem-gate`. It composes the exact candidate plan under one process, lock,
workspace, and run log. A nested gate deadlocks on its parent's lock; a split
design needs the parallel receipt authority the manifest contract forbids.

Read `/dev-gate` before changing this Python composition and `/dev-just` before changing its
public dispatch. Do not move orchestration into recipes, workflow YAML, or a release script.

## One command owns the complete release

Capsem has exactly two release-facing Just commands:

```bash
just release-binaries <channel>
just release-profile <channel> <profile>
```

These are the sole release entrypoints for humans and checked-in automation.
Do not ask an operator to run a preparation command or a separate `just test`
first. Each Python release plan contains the complete plan used by `just test`
and has this non-negotiable order:

```text
just release-binaries <channel>
  1. validate release notes and fetch the fresh serialized channel source
     manifest read-only; fail immediately if the manifest has no staged
     channel/profile authority
  2. compose and execute the complete `just test` candidate plan in-process
  3. only after success: run the binary release script and dispatch binary CI

just release-profile <channel> <profile>
  1. compose and execute the complete `just test` candidate plan in-process
  2. only after success: invoke capsem-admin release for that channel/profile
  3. correlate and watch that exact profile workflow through terminal success
```

The complete gate runs from a private source generation. Prechecks, source-head
capture/reconfirmation, and publication target the originating checkout because
work authored only in the disposable copy would disappear. The source guard
stops publication if the checkout no longer has the recorded HEAD and bytes.

The complete executor is also kernel-isolated for the entire candidate graph:
Bubblewrap provides a loopback-only namespace on Linux and Seatbelt provides
the macOS boundary. Network access is not restored process-wide for release.
An authenticated helper created immediately before sandbox re-exec serves only
the explicitly marked manifest-resolution, exact-main confirmation/push, and
final dispatch actions. Its one-time mode-0600 metadata is deleted before plan
work; every brokered command remains in the owning `GuardedRunner`, step log,
and run journal. Never use `outside_sandbox=True` inside the candidate modules.

`just fast-test` remains useful public developer feedback. It *is* the exact
private `_test-fast` module used by `just test` and release CI, including YAML
and source syntax, every source/release contract, Clippy, Python lint/type
checks, JavaScript checks/builds, and blocking Rust/Python/JavaScript
vulnerability audits. It is still not release qualification and must never
replace `just test` in either release command.

`just test` must be the first consequential command. Cheap read-only checks may
precede it so missing notes, a missing serialized channel source, wrong-case
paths, invalid workflow syntax, and similar deterministic failures stop before
hours of local work. The binary preflight must fetch the mutable manifest/source
fresh and may not bootstrap profile state. If the staged channel/profile source
does not exist, the operator must use `release-profile` first. If `just test`
fails, the release command must stop before stamping versions, changing tracked
files, committing, tagging, pushing, authoring a shared manifest, or dispatching
a workflow. Test this fail-stop behavior by executing the public recipes with
fake downstream commands; inspecting recipe text alone is insufficient.

After that gate succeeds, both commands run the same checked-in source guard.
It requires the clean `main` HEAD captured before `just test`, then
fast-forward-pushes that exact tested HEAD when it is ahead of `origin/main`.
It refuses a changed HEAD, dirty tree, divergence, or force-push. Only after
this guard may binary stamping or profile dispatch begin.

Do not introduce a skip flag, release-only reduced gate, preparation recipe,
environment-variable bypass, or direct checked-in caller of:

- `scripts/release-binaries.py`;
- `capsem-admin release` for a first-party public profile;
- `release.yaml` or `release-assets.yaml`.

Daily nightly automation calls `just release-profile nightly <profile>` once
for each selected profile and then `just release-binaries nightly`. It never
dispatches either downstream workflow directly. Direct GitHub UI dispatch is
not the documented or tested release path.

Each command owns one artifact family. There is no combined release command.
The commands may run sequentially when a profile requires new code, but neither
may rebuild the other command's artifact family.

`config/public-surface.toml` locks this command surface. Treat any change as an
explicit product/API decision.

## Local proof and release-CI composition

Local `just test` is the whole-world proof. Release commands run it in full
before any release side effect, then CI reuses the same private modules against
the manifest-selected complementary artifact family.

`just test` is the complete local CI-equivalent proof, not a smaller developer
smoke test. Before any Docker/Colima, bootstrap, package, profile, asset, or VM
work, it runs the independently executable `_test-fast` module. It then
rebuilds every package and every checked-in profile and runs all six checked-in
modules:

- `_test-fast`
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

The workflow must download and digest-check those immutable inputs and
materialize every locked language dependency before calling `_test-*`.
Directly invoked modules enter the same host-kernel sandbox themselves; a cold
runner discovering a missing dependency after entry is a workflow preparation
bug, not permission to fetch during qualification.

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

`scripts/check-release-workflow.sh` is platform-aware without weakening the
publication boundary. Linux reports absent Apple signing material and CI-owned
`cargo-sbom`/`cdxgen` as not applicable; it never fabricates a key or installs a
secret-dependent macOS toolchain. Present tools are validated, and macOS still
fails closed when the signing key is absent or malformed.

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

`capsem-admin release` supplies a unique dispatch identity, finds the workflow
run with that exact identity, and watches it with failure propagation. The
public command does not report success merely because GitHub accepted a queued
dispatch; this preserves serialized profile-then-binary ordering even when the
same channel already has pending work.

If the public package is too old, publish the immutable profile artifacts and
persist the staged source-manifest state, but do not deploy that incompatible
pairing. Other profiles, channels, packages, and binaries remain untouched.

The nightly lane always rebuilds its selected profile assets. The prior-run
artifact resolver is a stable retry mechanism only; using it for nightly would
turn the daily asset build into a no-op and lose the hermetic reproducibility
signal.

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

Daily nightly automation calls this same binary command path after the profile
commands terminate and queues behind other nightly release work. The binary
workflow always rebuilds and runs native install, functional, Winterfell,
IronBank, and glow-up proof. If the version tag is already immutable, the
correlated workflow runs with publication disabled; fresh Apple signing and
notarization timestamps are not byte-identical and may never overwrite an
existing release. A new version identity takes the normal publish-and-activate
path. Stable uses the same command explicitly and the same quality gates, and
has no schedule.

`release-nightly.yaml` owns only sequencing: selected profile commands run with
`max-parallel: 1`, each waits for its exact run, and the binary command runs
after the profile matrix terminates even if a profile failed. The scheduler's
`capsem-nightly-release-scheduler` lock prevents overlapping orchestrators; it
does not replace the downstream workflows' shared
`capsem-release-nightly` transaction lock.

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

## Published artifacts are load-bearing

A published manifest points at real storage, and some of it lives on GitHub
releases. Before deleting or retiring **any** published release, resolve what
the live manifests actually reference:

```bash
curl -s https://release.capsem.org/assets/<channel>/manifest.json \
  | grep -o 'releases/download/[^/]*' | sort -u
```

Check every manifest the catalog lists, not just `current` — a `supported` or
`deprecated` manifest keeps its own users alive. Both the VM assets **and** the
binary package can be hosted this way; assuming only one is a good way to break
the install path while believing you preserved it.

## Verify content, not status codes

**HTTP 200 is not proof that a resource exists.** The release site answers a
missing manifest with its SPA fallback: `200 OK` and an HTML body. Any check
that tests the status code passes while the manifest is absent.

Validate the bytes: parse the JSON, confirm the expected channel and version,
and verify the digest the catalog claims matches what was served.
`scripts/check-release-site-contract.py` does this and fetches every artifact a
manifest references, verifying size and sha256.

It runs at deploy time and, via `live-channel-watch.yaml`, daily and on demand.
The watch exists because the deploy gate can only notice a broken channel while
publishing a new one — anything that breaks an already-published channel from
outside a deploy (deleted release, artifact aged out by retention, CDN
misbehaviour) is otherwise invisible until the next release, and users meet it
first.

Run it by hand whenever you need to answer "is the channel healthy right now?":

```bash
uv run python scripts/check-release-site-contract.py \
  --base-url https://release.capsem.org --channel stable --attempts 1
```

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

### Diagnostic continuation is not release continuation

Use **diagnostic continuation** only to reach a late failure after a failed
non-release candidate. It may combine earlier outputs with current source:
zero means the segment passed, not that current source completed `just test`.
The transitional CLI spells this:

```bash
uv run capsem-gate runs last --failed
uv run capsem-gate candidate --prefix <retained-prefix> --from <failed-step>
```

Call it diagnostic continuation despite those legacy names. The named step
runs; predecessors are carried. Match prefix/frontier to the preceding failed
run, preserve its ID, and read `carried` as reused evidence, not a new `ok`.
Both release commands must reject these flags. Never use the result to stamp,
tag, push, dispatch, activate, or qualify. After the fix, rerun the public
release command from the beginning; only its clean complete plan may publish.

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
`just release-binaries` must validate that this section contains publishable
notes before it starts `just test`, and the binary release script must recheck
before version mutation. Never defer release-note validation until after the
complete local gate or source push. Profile releases are independent and do not
require binary changelog text.

Binary and profile versions are orthogonal:

- binary: the Capsem package/application version;
- profile: the immutable channel/profile publication identity derived and
  authored by `capsem-admin`.

Do not infer that a profile change requires a binary rebuild, or that a binary
change requires rebuilding any profile.

### Semver is mandatory, and each profile versions independently

Every version in the release system is strict semver `MAJOR.MINOR.PATCH`:

- the Capsem binary, whose patch increments -- it is **not** a timestamp;
- every profile revision, first-party and corp-authored alike;
- `min_capsem_version` / `max_capsem_version`, which bound the **binary** and
  are a separate axis from the profile's own revision. A profile at `0.3.2` may
  require capsem `>= 0.6.0`; those numbers are unrelated.

Profiles are orthogonal, so each carries its own revision and advances on its
own schedule. `code` moving to `0.7.0` says nothing about `co-work`. A release
spanning profiles at different revisions has no single version to name and
collapses to a `profiles-<hash>` identifier; that identifier names a set, not a
version, and is deliberately exempt from semver.

`capsem-admin` enforces this: `parse_profile_revision` rejects anything that is
not semver, and `ensure_revision_advances` rejects a revision that does not move
past what is already published. Both run before a release is authored, so a corp
operator meets the same rule.

This replaced a date-plus-counter scheme (`2026.06.08.9`) that could not order
releases. The date recorded when someone last edited the field rather than when
the assets were built, so a July build shipped wearing a June date; the counter
counted hand-edits, so revisions existed that were never published. Text
comparison also ranks `0.10.0` below `0.9.0`. Never reintroduce a version whose
components are dates, timestamps, or build counters.

## Commit discipline

1. Include the appropriate `CHANGELOG.md` entry for user-visible changes.
2. Stage files explicitly.
3. Use conventional commit subjects.
4. Never stage private release material, certificates, keys, tokens, or
   local-only credentials.
