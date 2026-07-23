---
name: release-process
description: Capsem release process, CI pipeline, Apple code signing, notarization, documentation site, and post-release verification. Use when preparing a release, debugging CI failures, working with Apple certificates, updating the documentation site, or cutting a new version. Covers the full release lifecycle from pre-release checklist through post-release verification.
---

# Release Process

## Public command discipline

`just test` is the sole release-qualification command. There are no public
`prepare-release`, `qualify-release`, `cut-release`, `release`, `install`, or
`test-*` recipes. Candidate/tag/publication mechanics remain workflow-owned;
native install proof remains glow-up-owned. Do not recreate a parallel Just
release path.

`config/public-surface.toml` locks the exact Just, Capsem CLI, and service HTTP
surfaces. Updating the ledger requires explicit product/API approval and must
be reviewed as a public contract change.

## Pre-release checklist

```bash
just doctor                    # Check tools
scripts/preflight.sh           # Validate Apple certs for CI
just test                      # ALL tests: unit + integration + cross-compile + bench
```

The checklist is developer feedback only. It is never release authorization.
For every stable and nightly release, `release-qualification.yaml` must run the
complete `just test` recipe in CI on the exact versioned, untagged candidate
commit. A final immutable tag must not exist until that run succeeds.
`release.yaml` then verifies the successful qualification's exact `headSha`
before package work. Never substitute a partial Rust/frontend/coverage job, a
previous or nearby green commit, a matching display title with another SHA, or
a local agent-run gate. CI is authoritative because agent-reported local
evidence is not trusted release proof.

Temporary hosted-CI exception: `just test` runs once on Linux because
GitHub-hosted macOS cannot expose the nested Virtualization.framework support
Capsem and Colima require, and no physical macOS runner is registered. Keep the
exception explicitly commented in `release-qualification.yaml`. The tagged
workflow must fail in preflight unless that exact qualification passed; macOS
and Linux package jobs fan out only after preflight. Restore a parallel macOS
full gate once a physical runner exists.

`just test` includes Winterfell/MCP persistence, the four-VM concurrency
canary, IronBank, integration and injection, benchmarks, cross-compilation,
Docker/systemd install tests, and on an Apple Silicon Mac an exact-package
install/glow-up in a disposable Tart guest. Run it exactly once in the release
workflow, not again after packaging. The exact final `.pkg`/`.deb` must
then be installed on macOS and Linux to exercise the real native installer and
post-install scripts before publication. Notarization and public
channel-switch/upgrade glow-up verification remain additional requirements and
provide the end-to-end deployed-release proof. Never replace the full gate
with doctor, Winterfell, smoke, or another selected subset.

Locally, the same complete gate is paid once per ready candidate—not after
every few-line repair. Use focused red/green tests and the relevant clean Linux
container proof while editing, then commit the complete candidate before the
one complete local `just test`. The wrapper refuses a dirty tree, records
`HEAD`, runs the internal candidate gate, and fails if either `HEAD` or the
working tree changes. The SHA printed at success is the local Apple VZ evidence
and the only SHA that may be pushed for qualification. A subsequent production
or gate change creates a new candidate and therefore needs a new complete run.

Stamp the forward-only version before that complete local gate. The real macOS
and Linux packages, host SBOM, installer metadata, and binary-version checks
built by `just test` must contain the exact version that will be committed,
qualified, and tagged. Version/changelog preparation is an explicit candidate
operation, not a public Just recipe.

Automatic benchmark recordings from `just test` belong under
`target/test-benchmarks/`, which is ignored and disposable. Intentional
historical benchmark publication uses the owning pytest/benchmark tools and an
explicit review; there is no benchmark convenience recipe. Never weaken the
clean-tree invariant by allowlisting benchmark source paths.

Rust is pinned to `1.97.1` in `rust-toolchain.toml`, every workflow toolchain
step, the host-builder image, and bootstrap. Bump every surface together in a
deliberate monthly toolchain PR and resolve new lint fallout there. Do not use
`stable`, `latest`, or an independently floating Docker compiler. Workspace
lints deny `dbg_macro` and `todo` so debugging placeholders cannot ship.

`cargo audit` is an external-clock security signal, not a per-diff compiler
gate. The scheduled/manual `security-audit.yaml` workflow owns the blocking
RustSec result and gets a named owner and remediation. The local complete gate
reports an audit failure loudly but does not invalidate an otherwise unchanged
release candidate solely because a new upstream advisory appeared.

Treat disk capacity as a release resource on both macOS and Linux. Never copy
an already-built multi-gigabyte immutable VM/package cohort into a second
same-filesystem staging tree late in qualification. Use hardlink-first staging
for immutable bytes, retain an executable cross-filesystem copy fallback, and
cover both paths with a constrained-disk regression that fails an accidental
copy with `ENOSPC`. Measure and report capacity before expensive artifact lanes
and again before the final installer/glow-up tail; discovering deterministic
disk exhaustion after the KVM and package rails is a qualification-harness bug.

Keep release-harness bootstrap checks fail-fast inside that canonical recipe.
Before expensive audits/builds/VMs/package assembly, Stage 0 must build the
clean Linux install image and prove its container-owned uv environment can run
`python -m pytest --version`. Contract-test that ordering. This catches host
virtualenv leakage and missing runner dependencies in seconds while retaining
the complete Docker/systemd install suite later in `just test`; the bootstrap
proof is never accepted as a substitute for install E2E.

Before paying for qualification, run the exact package materializer on macOS
and in the Linux host-builder against the live public channel URL, using every
release architecture and the complete checked-in profile catalog. The public
channel document is a release graph (`profiles`/`packages`), while local asset
inputs may still use the legacy `assets.current`/`assets.releases` schema; the
shared materializer must accept both intentionally and reject every incomplete
or unknown shape. Every Python reader of `release.capsem.org` must send an
explicit Capsem user agent. Cloudflare rejecting bare `urllib` while `curl`
succeeds is a client-contract failure that must be reproduced by an adversarial
local HTTP server and rejected by the fail-fast source guard.
`release-qualification.yaml` must enforce that contract as a cheap two-platform
job (macOS arm64 and Linux x86_64) on the exact candidate SHA. The canonical
`just test` job must depend on it, so a Cloudflare HTTP-policy or live manifest
schema failure cannot consume the multi-hour qualification budget. A local
focused test is necessary but is not a substitute for this remote preflight.

## Installer outcome gate

The installer exists to leave a working Capsem installation, not merely to
download a package, launch Installer.app, or return exit code zero. Treat the
following as separate, non-substitutable release gates:

### CI exact-artifact gates

- **macOS CI exact-package proof:** build the exact publishable `.pkg`, sign,
  notarize, and staple it, then install that same file with
  `sudo /usr/sbin/installer -pkg <pkg> -target /`. Assert the installed app,
  complete host-binary cohort, package version, manifest metadata, service
  registration, and launchable public CLI surfaces before uploading it.
- **Linux CI installed-product proof:** build the exact publishable `.deb` for
  every supported release architecture, install that same file on a clean
  native runner, and assert package metadata, the complete installed binary
  cohort, exact version agreement, service startup, and a functional Capsem
  command. Where the CI runner exposes KVM, prove `capsem shell` can start a
  guest and execute a deterministic command inside it. The current arm64
  hosted runner does not expose `/dev/kvm`, so arm64 must still pass the exact
  package/service proof while x86_64 owns the mandatory guest-shell proof.
- Publication must depend on both platform jobs. A skipped, optional,
  `continue-on-error`, mocked, source-layout, or inspect-only result does not
  count as release proof. Signing, notarization, file existence, and package
  expansion are necessary checks, but none substitutes for installing the
  artifact.
- Every installed-product rail must run
  `scripts/verify-installed-release.py` before accepting the install. That
  verifier must compare the installed manifest byte-for-byte with the selected
  manifest URL, require all manifest-declared profiles ready, validate the one
  canonical metadata sidecar including install/refresh/check state and package
  version, and reject every legacy origin/check/cache path. Do not replace this
  with ad-hoc status greps in an individual workflow.

### Public installer gates

- The public `curl -fsSL https://capsem.org/install.sh | sh` path must select
  the expected package from the release manifest, verify its declared byte
  size and SHA-256, synchronously apply it through the native package manager,
  and return failure when any step fails.
- After deployment, Linux CI must run that live command in a clean supported
  environment and repeat installed version, binary-cohort, service, and
  functional-command assertions. Parser tests and command stubs do not count
  as this gate.
- The installed `assets/manifest.json` must be the exact verified manifest
  document selected from the channel. Package postinstall and update code must
  not rewrite it into a reduced runtime schema or discard package binaries,
  profile descriptions, image records, ABOM/OBOM, software inventory, or host
  SBOM evidence. Runtime adapters may derive an in-memory boot view only.
- Installed manifest state has exactly one sidecar:
  `assets/manifest-metadata.json` with schema
  `capsem.manifest_metadata.v1`. It owns the manifest URL, channel/lock,
  package/install/refresh/check timestamps, checked URL and digest, validation
  result, and update comparison. Do not create a separate origin file, update
  check file, source-keyed cache directory, or UI-specific release cache.
- `GET /system/status` is the single installed-status contract. It returns the
  exact parsed `manifest.json`, exact parsed `manifest-metadata.json`, live
  profile readiness, corp state, and update comparison. `capsem status` and
  About Capsem must consume that same endpoint; the UI must not synthesize
  publication state or fetch a parallel profile/evidence status source.

### Stateful channel glow-up gate

The glow-up name is earned only by exercising one installed product through
state transitions. Fresh installs with different `CAPSEM_CHANNEL` values do
not prove channel switching and must never satisfy this gate.

- Run the compiled, package-installed CLI with `capsem update --channel`; a
  source inspection, fixture-only resolver, or direct `--manifest` substitution
  does not count as a public-channel switch.
- On the same Linux installation, prove stable -> nightly -> stable for VM
  assets and prove the installed manifest metadata records the exact selected
  channel, manifest URL, and correlated update-audit event each time.
- Prove a verified nightly package upgrade and a verified stable package
  downgrade through the native package manager. Linux downgrade application
  must use `apt-get --allow-downgrades`; both directions must leave the full
  binary cohort and service healthy.
- Move that installation to an explicit corporate manifest, persist
  `channel_kind=corporate` and `channel_locked=true`, refresh it successfully,
  then prove attempts to select stable, nightly, or a different corporate
  manifest fail before network or package mutation. A machine may enter corp
  but cannot leave corp through self-update.
- Verify the channel catalog record and selected manifest SHA-256 and BLAKE3
  before writing cache, assets, origin, or package state. Tampered manifests
  must fail nonzero and preserve the prior installed state.
- CI must run this stateful glow-up on the built Linux package.

### Accepted macOS VZ proof boundary

The complete local `just test` on the exact clean, versioned candidate is the
authoritative Apple Virtualization.framework guest-shell proof.
`scripts/macos_release_glowup.py` first installs the exact package in a disposable Tart Mac,
then extracts that same package on the physical host and boots a guest from the
packaged binaries and profiles. This split is required because Tart macOS
guests explicitly reject nested virtualization. The tagged
GitHub-hosted macOS job separately builds, signs, notarizes, staples, installs,
and verifies the exact publishable `.pkg`, but hosted runners cannot repeat the
VZ guest path because nested virtualization is unavailable.

The local package is intentionally unsigned. Its postinstall ad-hoc signs the
installed Mach-O payload with the required entitlements, so local qualification
needs no release certificate, private key, or temporary keychain. Developer ID
signing, notarization, stapling, Gatekeeper verification, and installation of
that final signed artifact remain owned exclusively by the tagged publication
workflow.

The accepted release risk is explicit: the published `.pkg` is not installed
again on a physical Mac for a second VZ guest-shell run after publication. Do
not claim that missing post-publication combination as evidence; record the
locally tested candidate SHA and the successful exact-package hosted macOS job.

## Release graph and channel publishing

Read `references/release-graph.md` for the asset manifest and channel graph
contract, `release-assets.yaml` behavior, the four disjoint CI lanes, and the
live release activation order. Load it before touching manifest generation,
channel assembly, or any release workflow's publishing tail.

## Cutting a release

### Release history discipline

Release history is forward-only. Once a commit or tag has been pushed, do not
amend it, force-push it, or force-move the tag to "save" that release. That
makes the release harder to audit and can leave CI, GitHub Releases, and local
checkouts disagreeing about what was actually shipped.

- Never use `git commit --amend`, `git push --force`, `git push --force-with-lease`,
  `git tag -f`, or a forced tag push for a release that has already left the
  machine.
- If an untagged candidate fails qualification, land a normal follow-up commit
  on top of `main` and requalify that new SHA without minting any tag. If a
  failure happens after a final tag exists, stamp a new unique version, create
  a new forward tag, and leave the old tag untouched.
- Cancel superseded failed CI runs when useful, but leave the historical commit
  and tag alone. The goal is a clean next release, not rewriting the failed one.
- Do not reuse a version string or tag name. For the `1.2.{unix_timestamp}`
  release line, choose a later timestamp and let the old tag remain historical.

### Prepare and remotely qualify an untagged candidate

```bash
just test
git push origin HEAD:main
gh workflow run release-qualification.yaml --ref main \
  -f "sha=$(git rev-parse HEAD)" -f channel=stable
```

Prepare the version and changelog explicitly, commit the ordinary candidate,
then run the complete local gate on that clean `HEAD`. It must not create a
tag, GitHub Release, or channel mutation. Push that ordinary commit, then
dispatch the canonical Linux qualification workflow. If qualification fails,
add a normal forward fix commit and qualify the new candidate. Do not mint
failure tags or stamp a new version merely to obtain another CI attempt.

### Mint the immutable tag after qualification

```bash
python3 scripts/check-release-qualification.py \
  --sha "$(git rev-parse HEAD)" --channel stable
git tag "v$(sed -n 's/^version = \"\\([^\"]*\\)\"/\\1/p' Cargo.toml | head -1)"
```

Tagging performs no stamping and creates no commit. First compare `HEAD` with
`origin/main`, require a successful completed qualification, and reject
existing local or remote tag names. Missing, pending, failed, or malformed
qualification results are hard failures.

### Manual publish

1. Confirm the release tag does not already exist remotely:
   `git ls-remote origin "refs/tags/vX.Y.Z"`
2. Confirm exact-SHA qualification again:
   `python3 scripts/check-release-qualification.py --sha "$(git rev-parse HEAD)" --channel stable`
3. Push the immutable tag: `git push origin vX.Y.Z`
4. Dispatch the one channel workflow:
   `gh workflow run release.yaml --ref vX.Y.Z -f tag=vX.Y.Z -f channel=stable`
5. Watch that exact run with `gh run watch --exit-status`.

There is deliberately no Just release wrapper. Publication is not a second
test gate, and hiding tag selection, workflow dispatch, polling, and retry
semantics behind a large recipe created a fork from the canonical `just test`
path. Qualification, tag creation, and final workflow dispatch remain explicit.

Never reuse or move a tag. Always increment the version number, and always tag
forward.

Before candidate qualification, run
`scripts/check-hardcoded-release-selections.sh` through `just test`. It rejects
named profile selection in UI/tray/CLI/MCP request paths, one-profile release
materialization, qualification that is not channel-bound, and native installer
fallbacks to stable/nightly. Keep the vocabulary list current; it intentionally
includes `code`, `co-work`, `cowork`, `terminal`, the known `termional` spelling,
and `gui` so future profile renames cannot bypass the guard during migration.
This fail-fast guard must remain runnable in clean qualification Linux with
only Python's standard library; its focused contract deliberately removes
`rg` from `PATH` so a developer-only search dependency cannot burn another
full-gate attempt.

### GitHub CLI release control

Use `gh` as the release control plane:

```bash
gh auth status
gh release list --limit 10
git ls-remote origin "refs/tags/vX.Y.Z"
git push origin HEAD:main
git push origin vX.Y.Z
gh run watch <run-id>
gh run view <run-id> --json status,conclusion,headSha,url
gh run view <run-id> --log-failed
gh release view vX.Y.Z --json name,tagName,isDraft,isPrerelease,assets,url
```

Before pushing a tag, confirm the tag does not already exist remotely and the
exact candidate qualification succeeded. After pushing, dispatch the selected
channel and watch package proof and publication to completion. If candidate
qualification fails, diagnose it with `gh run view --log-failed`, assign a
named owner, make a forward fix, and requalify without creating any tag. Red is
stop-the-line: do not merge over it and do not blindly retry an unchanged SHA.
A failure after tagging still requires a new forward version and tag; never
move the old tag.

## CI pipelines

Candidate qualification is dispatched with `{sha}` before a tag exists:

```
release-qualification.yaml: exact untagged SHA ──> just test

release.yaml: verified tag/SHA/qualification ──> build-app-macos (exact .pkg install) ──┐
                                         └─────> build-app-linux (exact .deb installs) ─┴──> create-release
                                               create-release + channel preview ──> verify public candidate ──> advance channel
```

| Job | Runner | Needs | Purpose |
|-----|--------|-------|---------|
| `qualification` | ubuntu-24.04 | -- | Exact-SHA canonical `just test`, read-only and untagged |
| `preflight` | macos-14 | -- | Verify tag identity, exact qualification, Apple cert, Tauri key, notarization |
| `build-app-macos` | macos-14 | preflight | Build, sign, notarize, staple, Gatekeeper-check, install, and verify exact `.pkg` |
| `build-app-linux` | ubuntu arm64 + x86_64 | preflight | Build `.deb` packages, install and verify each exact artifact, and prove a guest shell on KVM |
| `create-release` | ubuntu-latest | build-app-macos, build-app-linux | Publish the install-tested `.pkg`, mandatory `.deb` files, and host SBOM |
| `verify-release-candidate` | ubuntu-24.04 | create-release, assemble-release-channel | Verify public package URLs, SHA-256/BLAKE3, and `install.sh` against the candidate manifest |
| `deploy-release-channel` | reusable | verify-release-candidate | Advance the user-discoverable channel only after candidate verification |

The qualification job completes once, then the platform builds and exact
artifact install gates fan out. A public GitHub Release is inert package
storage until `release.capsem.org` points clients at it; draft assets cannot be
verified through public download URLs. Create the public storage release,
verify every candidate URL and installer selection, and advance the channel as
the final publish moment. Never advance the channel from a failed or unverified
candidate.


### CI invariants (hard-won lessons)

Read `references/ci-invariants.md` before editing any release workflow. It
holds the Ironbank local/CI parity rule (every portable release gate must be
owned by `just test`) and every burned-release lesson: AppImage's 14 failed
releases, Xcode pinning, musl toolchain flags, platform gating, cdxgen
pinning, Docker prune races, disk-capacity staging, and more. Skipping it
repeats releases that already failed once.


| Gate | What |
|------|------|
| Unit tests | `cargo llvm-cov` with coverage |
| Cross-compile | capsem-agent for aarch64 + x86_64 musl |
| Frontend | `pnpm run check && pnpm run build` |
| capsem-doctor | Boot VM, run full diagnostic suite |
| Integration | Boot VM, exercise all 6 telemetry pipelines |
| Benchmark | Boot VM, run capsem-bench |


## Apple code signing and CI secrets

Read `references/apple-signing.md` when touching signing, notarization, the
p12 certificate, release secrets, or Cloudflare release-channel prerequisites.
The p12 legacy-3DES gotcha and the Apple agreement 403 playbook live there.

## Post-release verification

Read `references/post-release-verification.md` after any publication: public
package verification, the stateful glow-up, the two-cohort binary transition
proof, and the demo-facing macOS installer proof.


The product website uses Astro Starlight. Docs live in `docs/src/content/docs/`.

### Writing style
Tight and to the point. One topic per page. Tables over prose for configs and test cases. No filler.

### Structure
- `docs/src/content/docs/<category>/<topic>.md`
- Categories: `security/`, `testing/`, `releases/`, `architecture/`
- Frontmatter: `title` and `description` required. `sidebar: { order: N }` for ordering.

### Release pages
- Path: `docs/src/content/docs/releases/<major>-<minor>.md` (hyphens, not dots)
- Each page consolidates all patch releases for that minor
- Higher `sidebar.order` = newer = listed first

### Dev workflow
```bash
cd site && pnpm run dev     # localhost:4321
cd site && pnpm run build   # Production build
```

### Keep docs in sync
When features change (settings, CLI flags, MCP tools, security invariants, benchmarks), update the corresponding doc page. When cutting a new minor, create a new release page.

### Update benchmarks before release

Run the host-side benchmarks to generate versioned data files and update the results page:

```bash
# Generate benchmarks/fork/data_{version}.json and benchmarks/lifecycle/data_{version}.json
uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py -xvs

# Update docs/src/content/docs/benchmarks/results.md with new numbers
# (manual -- copy from the benchmark summary tables)
```

Benchmark data files in `benchmarks/` are committed to git for historical tracking. The `test_fork_benchmark` gates ensure fork stays under 500ms and images under 12MB -- these must pass before release.

## Changelog

Keep a Changelog format in `CHANGELOG.md`. Every user-visible change gets an
entry under `## [Unreleased]` using the standard added, changed, deprecated,
removed, fixed, and security groups.

## Versioning

Binary and asset versions are **orthogonal**:

- **Binary**: `1.3.{unix_timestamp}` for the current release line. Select it
  before committing and qualifying the candidate; tagging never changes it.
  Set `CAPSEM_RELEASE_VERSION=x.y.z` when an exact preselected stamp is needed.
- **Assets**: `YYYY.MMDD.patch` -- derived by `capsem-admin manifest generate` from the build date

Three files hold the binary version (kept in sync by `_stamp-version`): `Cargo.toml` (workspace), `crates/capsem-app/tauri.conf.json`, `pyproject.toml`.

The v2 manifest links them via `min_binary` (oldest binary for these assets) and `min_assets` (oldest assets for this binary). See `/asset-pipeline` for manifest format.

## Commits

1. Include `CHANGELOG.md` update in the same commit
2. Stage files explicitly (no `git add -A`)
3. Conventional messages: `feat:`, `fix:`, `chore:`, `docs:`
4. Author: Elie Bursztein <github@elie.net>
5. No `Co-Authored-By` trailers
6. Never stage private release material (`private/`, `capsem-private.zip`,
   `graphics.zip`, certificates, keys, tokens, or local-only demo credentials)
