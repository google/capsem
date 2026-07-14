---
name: release-process
description: Capsem release process, CI pipeline, Apple code signing, notarization, documentation site, and post-release verification. Use when preparing a release, debugging CI failures, working with Apple certificates, updating the documentation site, or cutting a new version. Covers the full release lifecycle from pre-release checklist through post-release verification.
---

# Release Process

## Pre-release checklist

```bash
just doctor                    # Check tools
scripts/preflight.sh           # Validate Apple certs for CI
just test                      # ALL tests: unit + integration + cross-compile + bench
```

The checklist is developer feedback only. It is never release authorization.
For every stable and nightly tag, the globally serialized `release.yaml`
workflow must run the complete `just test` recipe again in CI on that exact
tag, and every build/publication/deployment job must depend on it. Never
substitute a partial Rust/frontend/coverage job, a previous green commit, or a
local agent-run gate. CI is authoritative because agent-reported local evidence
is not trusted release proof.

Temporary hosted-CI exception: `just test` runs once on Linux because
GitHub-hosted macOS cannot expose the nested Virtualization.framework support
Capsem and Colima require, and no physical macOS runner is registered. Keep the
exception explicitly commented in `release.yaml`. The macOS packaging job must
depend on that Linux full gate and fan out only after it passes. Restore a
parallel macOS full gate once a physical runner exists.

`just test` includes Winterfell/MCP persistence, the four-VM concurrency
canary, IronBank, integration and injection, benchmarks, cross-compilation, and
Docker/systemd install tests. Run it exactly once in the release workflow, not
again after packaging. The exact final `.pkg`/`.deb` must
then be installed on macOS and Linux to exercise the real native installer and
post-install scripts before publication. Notarization and public
channel-switch/upgrade glow-up verification remain additional requirements and
provide the end-to-end deployed-release proof. Never replace the full gate
with doctor, Winterfell, smoke, or another selected subset.

Keep release-harness bootstrap checks fail-fast inside that canonical recipe.
Before expensive audits/builds/VMs/package assembly, Stage 0 must build the
clean Linux install image and prove its container-owned uv environment can run
`python -m pytest --version`. Contract-test that ordering. This catches host
virtualenv leakage and missing runner dependencies in seconds while retaining
the complete Docker/systemd install suite later in `just test`; the bootstrap
proof is never accepted as a substitute for install E2E.

## Installer outcome gate

The installer exists to leave a working Capsem installation, not merely to
download a package, launch Installer.app, or return exit code zero. Treat the
following as separate, non-substitutable release gates:

### CI exact-artifact gates

- **macOS CI exact-package proof:** build the exact publishable `.pkg`, sign,
  notarize, and staple it, then install that same file with
  `sudo /usr/sbin/installer -pkg <pkg> -target /`. Assert the installed app,
  complete host-binary cohort, package version, manifest origin, service
  registration, and launchable public CLI surfaces before uploading it.
- **Linux CI installed-product proof:** build the exact publishable `.deb` for
  every supported release architecture, install that same file on a clean
  native runner, and assert package metadata, the complete installed binary
  cohort, exact version agreement, service startup, and a functional Capsem
  command. Where the CI runner exposes KVM, prove `capsem shell` can start a
  guest and execute a deterministic command inside it.
- Publication must depend on both platform jobs. A skipped, optional,
  `continue-on-error`, mocked, source-layout, or inspect-only result does not
  count as release proof. Signing, notarization, file existence, and package
  expansion are necessary checks, but none substitutes for installing the
  artifact.

### Public installer gates

- The public `curl -fsSL https://capsem.org/install.sh | sh` path must select
  the expected package from the release manifest, verify its declared byte
  size and SHA-256, synchronously apply it through the native package manager,
  and return failure when any step fails.
- After deployment, Linux CI must run that live command in a clean supported
  environment and repeat installed version, binary-cohort, service, and
  functional-command assertions. Parser tests and command stubs do not count
  as this gate.

### Final local macOS installed-product proof

GitHub-hosted macOS cannot prove the VM-backed shell path because nested
Virtualization.framework is unavailable. After the immutable release and
public installer are deployed, the agent must download the exact published
`.pkg` on a real supported Mac, verify its release digest/signature/notarization,
install it with `sudo /usr/sbin/installer -pkg <pkg> -target /`, verify the
installed app, binary cohort, exact version, service, and tray, then prove
`capsem shell` spawns a guest shell and executes a deterministic command.

This is an agent-executed acceptance gate, not a manual GUI click-through or a
handoff to the user. A manual GUI click-through does not count as release proof.
Do not call the release complete without the recorded Linux CI installed-product
evidence and final local macOS installed-product evidence. If either fails,
fix forward, cut a new immutable version, and run the entire forward-only release
again; never reuse or move the failed tag.

Release asset manifests are generated through `capsem-admin manifest generate`.
Do not publish or document alternate manifest writers. Runtime VM asset
integrity is BLAKE3 hash verification plus manifest origin/hash reporting; do
not resurrect local manifest-signing keys or `manifest-sign.pub` verification.

The public asset channel is generated by `capsem-admin assets channel build`
from `assets/manifest.json` into `target/release-channel/`, with the machine
manifest at `target/release-channel/assets/<channel>/manifest.json`. The stable
public manifest URL is `https://release.capsem.org/assets/stable/manifest.json`.
`capsem-admin` owns the machine artifacts only: root `channels.json`,
per-channel manifest JSON, profile-owned artifacts/evidence, `_headers`, and
`robots.txt`. Human HTML is built by the `release-site/` Astro app from those
generated JSON files, using
`CAPSEM_RELEASE_CHANNEL_DIST=/path/to/target/release-channel pnpm run
build:channel`; the overlay copies Astro's root channel list, per-channel
pages, and per-profile pages into the same deploy root before
`capsem-admin assets channel check` runs.

The public graph is hierarchical and signed by reference:

1. `channels.json` lists every channel, such as `stable` and `nightly`.
2. Each channel lists versioned manifest records with exactly one `status`
   enum value: `current`, `supported`, `deprecated`, or `revoked`.
3. Each manifest record carries `version`, `url`, SHA-256, BLAKE3, HMAC
   metadata. The only public manifest URL for a channel is
   `/assets/<channel>/manifest.json`. Manifest records are retained for
   auditability; retained manifest records are audit rows, not alternate fetch
   URLs. Removal means the record is absent from the channel list, not marked
   with a second status.
4. Each manifest lists package artifacts separately from the per-binary
   inventory. Packages are delivery containers such as `.pkg` and `.deb`.
   Binaries are executable files inside those packages, and every binary entry
   lives under its owning package with SHA-256, BLAKE3, version,
   `installed_path`, and SBOM component reference so enterprise allowlists can
   reason about executables directly. Do not publish manifest-level
   `binaries`; package ownership is the provenance.
5. Each manifest points to profiles. Profiles own their config files, profile
   images, ABOM/OBOM evidence, software inventory, and `min_capsem_version`.
   Profiles do not point at the selected Capsem binary; they only declare a
   minimum Capsem version when profile behavior requires newer client support.

Do not add a parallel release truth file. The root `channels.json`, selected
manifest, and profile-owned image/config/evidence files are the update
contract. The root channel catalog live on `release.capsem.org` is
`/channels.json`. Runtime checks consume manifest URLs only:
`CAPSEM_RELEASE_MANIFEST_URL=https://release.capsem.org/assets/stable/manifest.json`
for stable, and the same variable pointing at
`https://release.capsem.org/assets/nightly/manifest.json` for nightly until a
friendlier channel selector is added.
Do not add a separate release-channel source directory or hand-authored channel
manifest. VM asset releases must deploy `release.capsem.org` after producing the
asset manifest/evidence. The public contract is `release.capsem.org`; the large
immutable VM blobs may live in GitHub Releases or another blob store, but the
manifest must carry instantiated URLs and every public index/profile reference
must resolve through the graph. Binary releases are explicitly dispatched for
one immutable tag and one selected channel, update
only package metadata, per-binary metadata, host SBOM, and host attestations in
the selected channel manifest, preserve already-published profile images
instead of copying image blobs into the Pages dist, and deploy the channel
without rebuilding profile images.

The manual VM asset release entrypoint is `.github/workflows/release-assets.yaml`.
For `dry_run=false`, it first verifies that the configured
`CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN` can see the Pages project
serving `release.capsem.org`, so a bad release-site binding fails before profile image builds,
immutable GitHub asset publication, or provenance attestation. It builds assets,
generates `assets/manifest.json`, builds
`target/release-channel/`, renders the Astro release site from that generated
channel data, publishes changed profile image blobs to an immutable GitHub
Release tagged `assets-v<asset-version>` with arch-prefixed `vmlinuz`,
`initrd.img`, `rootfs.erofs`, `obom.cdx.json`, and
`software-inventory.json` artifacts, writes instantiated
artifact URLs into the manifest,
uploads the generated release site without VM blobs as the
`asset-channel-preview` artifact, and calls
`.github/workflows/release-channel.yaml` to deploy that generated site when not
running in dry-run mode. Before the asset delta check and channel build, it
preserves the live channel's `binaries` metadata in the generated asset
manifest so VM asset releases do not erase package hashes, host SBOM evidence,
or binary attestation state from `release.capsem.org`. Manual VM asset releases
do not accept or publish a binary-version override; binary release metadata is
owned by the parameterized binary rail. Live asset releases must publish GitHub build
provenance attestations for those five arch-prefixed VM asset subjects. In
dry-run mode the workflow must print the exact `gh release` commands it would
execute without publishing or attesting, and upload `asset-release-plan` with
the generated upload script for review when current VM blobs changed. Every run
must also upload `asset-release-delta` with the manifest comparison decision.
The delta emits both `asset_changed` and `asset_blobs_changed`: metadata-only
asset release changes, such as deprecating an older VM asset release, still
deploy the release channel without republishing immutable VM blobs, and
`asset-release-plan`, GitHub Release upload, and provenance attestation run
only when `asset_blobs_changed` is true. `build-ledger.log` and `B3SUMS` remain
debug evidence unless deliberately published as separate evidence artifacts.
The manifest artifact is diagnostic/source evidence only; release-channel deploys consume the
generated dist artifact after the Astro overlay so the root channel list,
channel pages, profile pages, JSON graph, and headers stay in lock-step. The
first channel publication may continue when the
previous `release.capsem.org/assets/<channel>/manifest.json` is unavailable;
the asset delta gate records `previous_manifest_unavailable` as changed so the
initial site can bootstrap. The first channel bootstrap may have no host binary
evidence yet because the binary rail has not recorded package
files, the canonical `capsem-sbom.spdx.json` host SBOM reference, or host
binary attestations; once binary files are published, missing host SBOM
evidence is release-blocking.
Later publications still compare against the live
previous manifest and skip deployment only when current VM blob hashes, asset release metadata, and manifest policy are all unchanged. Manifest policy includes channel-visible fields such as `refresh_policy`. After
Cloudflare deploys, the channel workflow must run
`scripts/check-release-site-contract.py` against `https://release.capsem.org`
and smoke-check `https://release.capsem.org/`, `/channels.json`, and
`/assets/<channel>/manifest.json` through the public custom domain before it
passes. The Python validator reuses the remote release readiness contract and
must validate the root channel catalog, selected manifest,
profile-owned image/config/evidence documents, package metadata, per-binary
metadata, BLAKE3/SHA-256 content, attestation references, and cache headers
rather than only checking that files exist. The smoke must reject stale public
HTML: the root and channel pages must show the same generated timestamp,
manifest URL, manifest version, package inventory, per-binary inventory,
profile revision, image artifact URLs, and evidence URLs
as the fetched JSON graph. It must also verify every manifest record status and
digest in `channels.json`, including deprecated and revoked records, so
metadata-only changes cannot leave the public release history stale. It must
fetch profile-owned artifacts and verify BLAKE3, SHA-256, byte size, software
inventory, config entries, image artifacts, ABOM/OBOM evidence, and absence of
accidental bare paths. The smoke must
validate attestation subjects and predicate URLs after the evidence document
shape passes.
Host SBOM evidence is incomplete unless
`github_attestations_host_sbom` is present and points at the published
`capsem-sbom.spdx.json` evidence and covers every published host package
subject. VM asset attestations are incomplete unless
`github_attestations_vm_assets` is present and its `predicate_url` points at the
published VM OBOM evidence for the current asset release. It must also verify public `Cache-Control`
headers: mutable pointers (`/`, `/channels.json`, and
`/assets/<channel>/manifest.json`) stay `no-cache, must-revalidate`, while
immutable asset and profile release artifacts stay
`public, max-age=31536000, immutable`.

Docs and marketing deploy independently from binary, VM asset, and asset-channel
release rails. `ci.yaml` runs `docs-build`, `site-build`, and
`release-site-build` under `pr-gate` so broken docs, marketing, or
release-channel pages cannot merge while branch protection still requires one
stable status. `docs.yaml` and `site.yaml` deploy and smoke only on every push
to `main`: `https://docs.capsem.org/` plus `/getting-started/` for docs, and
`https://capsem.org/` for marketing. Those smokes are deploy checks only; they
must not depend on release tags or VM asset publication.

### Release graph CI lanes

Keep the lanes disjoint:

- The binary lane is the manually dispatched, globally serialized
  `release.yaml` path. It accepts one immutable tag and one `stable` or
  `nightly` channel. It may update packages, per-binary inventory, host SBOM,
  and host attestations for only the selected channel. It must not rebuild
  profile images, mutate profile image metadata, or update the other binary
  channel. Nightly is the daily binary iteration channel and stable is
  promoted on the weekly cadence.
- The profile lane is the manual `release-assets.yaml` path. It may update one
  channel/profile image set, profile config files, ABOM/OBOM evidence, profile
  catalog metadata, and matching manifest digests. It must not mutate packages,
  per-binary inventory, other profiles, or other channels.
- The channel discovery lane owns `channels.json`, manifest selection, status
  enum validation, SHA-256/BLAKE3 validation, and cache headers. Revoked manifest
  records stay in the audit graph but are never selected.
- The deploy lane is reusable `release-channel.yaml`; binary and profile lanes
  hand it a generated release-site artifact instead of deploying directly.

The final stable-to-nightly switch acceptance gate must start on stable, verify
the stable graph and co-work profile image/config/evidence, switch to nightly
by manifest URL, verify nightly binary and co-work profile data, prove stable
cached data is unchanged, switch back to stable, and reject any package,
per-binary, profile image, config, or evidence data crossing channels.
This stable-to-nightly acceptance proof is required before calling the release
graph complete.

## Live release activation order

Use this order when turning the 1.5 release rails on. Do not skip ahead because
later steps depend on earlier public state being true.

1. Merge the release-rail commits to `main` only after the pull request's
   expanded `pr-gate` passes.
2. Require only `pr-gate` in branch protection or active rulesets.
3. Provision the `release.capsem.org` Cloudflare Pages project and DNS for the
   generated `target/release-channel/` artifact.
4. Run `uv run python scripts/check-remote-release-readiness.py`; continue only
   after unpublished commits, remote fail-closed `pr-gate` shape, branch
   protection, `release.capsem.org` DNS, public cache headers, and
   release-channel content all pass.
5. Run `.github/workflows/release-channel-staging.yaml` against the Cloudflare
   Pages staging branch. It builds the deterministic release-channel fixture,
   deploys the generated dist through `.github/workflows/release-channel.yaml`
   with a non-main branch and preview URL, and proves the release-channel deploy
   path without invoking VM asset builds or binary package builds.
6. Run the manual VM asset workflow as a dry run and review the
   `asset-release-plan`, `asset-release-delta`, and `asset-channel-preview`
   artifacts. For metadata-only asset release changes, review
   `asset-release-delta` and `asset-channel-preview`; no `asset-release-plan`
   is expected because there are no immutable VM blobs to republish.
7. Run `.github/workflows/release-binary-staging.yaml` and review the
   `binary-channel-dry-run-bundle` artifact. It records deterministic fake host
   package and `capsem-sbom.spdx.json` metadata into a copy of the live asset
   manifest, builds the release-site preview, and writes `proof.json` showing
   VM asset metadata was not changed. This is the safe binary dry-run path.
8. Push a new immutable `vX.Y.Z` tag, then explicitly dispatch the binary
   release rail with that tag and exactly one `stable` or `nightly` channel.
   The global concurrency group prevents channel deployment races. The binary
   lane updates only the selected channel and proves profile image metadata is
   unchanged.
9. Run the manual VM asset workflow live only after reviewing
   `asset-release-plan` when `asset_blobs_changed` is true, or reviewing the
   metadata-only delta and channel preview when only release-channel metadata
   changed; it must publish changed VM blobs, attest them, and deploy
   `release.capsem.org`.
10. Run installed update smokes for the signed macOS `.pkg`, Linux `.deb`, VM
   asset refresh, profile update path, and staged cross-surface update state.

## Cutting a release

### Release history discipline

Release history is forward-only. Once a commit or tag has been pushed, do not
amend it, force-push it, or force-move the tag to "save" that release. That
makes the release harder to audit and can leave CI, GitHub Releases, and local
checkouts disagreeing about what was actually shipped.

- Never use `git commit --amend`, `git push --force`, `git push --force-with-lease`,
  `git tag -f`, or a forced tag push for a release that has already left the
  machine.
- If a pushed release commit or tag fails CI, land a normal follow-up commit on
  top of `main`, stamp a new unique version, create a new tag, and push forward.
- Cancel superseded failed CI runs when useful, but leave the historical commit
  and tag alone. The goal is a clean next release, not rewriting the failed one.
- Do not reuse a version string or tag name. For the `1.2.{unix_timestamp}`
  release line, choose a later timestamp and let the old tag remain historical.

### Prepare release commit and local tag

```bash
just cut-release
```

Runs `test` (all tests including integration, cross-compile, benchmarks), then
bumps the version, stamps the changelog, creates the release commit, and creates
a local `vX.Y.Z` tag. It does **not** push. Push the branch and tag manually
after checking the local commit/tag.

### Manual publish

1. Confirm the release tag does not already exist remotely:
   `git ls-remote origin "refs/tags/vX.Y.Z"`
2. Push the release commit to `main`: `git push origin HEAD:main`
3. Push the immutable tag: `git push origin vX.Y.Z`
4. Dispatch and watch one channel workflow: `just release vX.Y.Z stable`

Never reuse or move a tag. Always increment the version number, and always tag
forward.

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

Before pushing a tag, confirm the tag does not already exist remotely. After
pushing, dispatch the selected channel and watch the release workflow to
completion. If CI fails, use
`gh run view --log-failed` to diagnose, make a forward fix, and cut the next tag.

## CI pipeline (release.yaml)

Explicitly dispatched with `{tag, channel}` and globally serialized:

```
preflight ──> test ──> build-app-macos (exact .pkg install) ──┐
                  └──> build-app-linux (exact .deb installs) ─┴──> create-release
```

| Job | Runner | Needs | Purpose |
|-----|--------|-------|---------|
| `preflight` | macos-14 | -- | Fail-fast: Apple cert, Tauri key, notarization |
| `test` | macos-14 | preflight | Unit tests + coverage, frontend, audit |
| `build-app-macos` | macos-14 | preflight, test | Build, sign, notarize, and staple `.pkg`, then install and verify that exact package |
| `build-app-linux` | ubuntu arm64 + x86_64 | preflight, test | Build `.deb` packages, then install and verify each exact matrix artifact |
| `create-release` | ubuntu-latest | test, build-app-macos, build-app-linux | Publish the install-tested `.pkg`, mandatory `.deb` files, and host SBOM |

The qualification job completes once, then the platform builds and exact
artifact install gates fan out. Publication cannot start unless all are green.

### CI invariants (hard-won lessons)

- **CI is a clean checkout.** If the build depends on a generated source file,
  either track it or regenerate it in CI before the consumer imports it. A local
  generated file hidden by `.gitignore` can pass local tests and fail immediately
  in GitHub Actions. The frontend `mock-settings.generated.ts` file is an example:
  `mock-settings.ts` imports it, so it must exist in a clean checkout or be
  generated by the workflow.
- **PR install E2E owns broad package contracts; release builds own exact
  artifact acceptance.** `ci.yaml` runs hermetic `test-install` before merge.
  The release workflow does not rebuild another debug package: each platform
  build installs and verifies the exact signed/notarized `.pkg` or release
  `.deb` it uploads. Postinstall hydrates through
  `capsem update --assets --manifest <URL>` for the selected channel; VM
  payload rebuilds live in the manual asset workflow.
- **Linux `.deb` self-updates stop stale helpers before replacement.**
  `scripts/repack-deb.sh` must include `scripts/deb-preinst.sh` as
  `DEBIAN/preinst`. That preinstall script runs
  `systemctl --user stop capsem.service` when a user systemd session is
  available, then kills the stale helper cohort before package replacement so
  old service/gateway/tray/process binaries cannot survive from old inodes.
  `scripts/deb-postinst.sh` owns symlink refresh, asset hydration, and service
  registration after replacement.
- **Clean-checkout proof belongs before tagging.** When fixing release-only
  failures, test the exact path a runner takes: fresh checkout, install deps,
  then focused checks (`pnpm -C frontend run check`, generated-config conformance
  tests, `pnpm -C frontend run test`, `pnpm -C frontend run build`) before the
  full release gate.
- **Manual VM asset releases use arch-prefixed blob names on release.capsem.org.**
  `capsem-admin assets channel build` writes the channel manifest to
  `assets/<channel>/manifest.json` and immutable blobs to
  `assets/releases/<asset-version>/<arch>-<logical_name>`. The v2 manifest keeps
  bare filenames in per-arch `arches` maps.
- **Manual asset CI uses justfile recipes.** `.github/workflows/release-assets.yaml`
  must call `just build-kernel` and `just build-rootfs`, not reimplement the
  builder commands. Drift between the justfile and CI caused v0.14.2-v0.14.4
  to ship without vmlinuz/initrd.img.
- **Manual asset releases build both kernel and rootfs.** The builder defaults
  to `--template rootfs` only. The kernel template must be built explicitly.
- **Asset CI needs the musl C toolchain, not just Rust targets.** The manual
  asset matrix must install `musl-tools` and pass
  `CC_aarch64_unknown_linux_musl=musl-gcc`,
  `CC_x86_64_unknown_linux_musl=musl-gcc`,
  `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc`, and
  `CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc` into the
  `just build-kernel` / `just build-rootfs` step. Crates such as `ring` compile
  C/ASM during guest binary builds; `rustup target add` alone is not enough.
- **App packaging cargo-tool installs must be retryable and independent.**
  GitHub-hosted runners can hit transient crates.io DNS timeouts while
  installing release tools. Do not install `tauri-cli`, `cargo-auditable`, and
  `cargo-sbom` in one `cargo install` command: one timeout discards all useful
  progress. Install each tool separately with `CARGO_NET_RETRY=10` and a small
  shell retry loop so a single registry lookup hiccup does not fail the release.
- **`Cargo.lock` is gitignored.** CI resolves a fresh lockfile each build. This means dependency versions can drift between builds. Acceptable for now but a reproducibility risk.
- **Three files hold the binary version.** `Cargo.toml` (workspace), `crates/capsem-app/tauri.conf.json`, `pyproject.toml`. `just _stamp-version` handles all three automatically. `just cut-release` and `just install` both call it.
- **Do not resurrect local VM manifest signing.** VM asset integrity is the
  profile manifest plus BLAKE3 hashes, manifest origin/hash reporting, and
  SBOM/OBOM/build-ledger evidence. Local `manifest-sign.pub` keys and minisign
  setup are security theater for this rail. Tauri updater signatures still use
  `TAURI_SIGNING_PRIVATE_KEY`; do not confuse that with VM asset manifests.
- **Do not make macOS CI depend on a Homebrew-only `flock` binary.** GitHub's
  macOS runners do not provide `flock`, even when developer machines do.
  Shared `just` execution locking must work with the checked-in
  `scripts/lib/exec_lock.sh` fallback: use `flock` when it exists and a Python
  `fcntl.flock` holder process otherwise. Keep `flock` out of `capsem-doctor`
  required tools unless the fallback is removed.
- **Treat the PR Python schema lane as a scoped contract gate, not the full
  Python coverage gate.** The macOS PR job intentionally runs
  `tests/test_*.py` so it does not boot VM suites; on a clean GitHub macOS
  runner that top-level subset reports about 88.67% coverage, so the workflow
  floor is 89%. The complete local `just test` Python stage still runs the full
  suite and keeps its 90% floor.
- **Do not execute artifact-dependent Python suites on a clean PR runner before
  creating their artifacts.** `tests/capsem-bootstrap/` needs real
  `assets/<arch>/` plus `assets/manifest.json`, and `tests/capsem-codesign/`
  needs built, signed host binaries. The PR macOS no-VM integration lane runs
  only suites without generated prerequisites and then import-collects every
  `tests/capsem-*/` suite; the full `just test` gate owns bootstrap/codesign
  execution after `_pack-initrd`/`_sign` have made the prerequisites real.
- **Do not run live KVM probes on GitHub-hosted PR runners.** Hosted ARM runners
  can expose `/dev/kvm` but still hang or behave inconsistently under test
  execution. PR Linux CI sets `CAPSEM_SKIP_KVM_TESTS=1` and runs
  `cargo test --no-run --all-targets` for the portable host crates: it compiles
  the KVM backend and Linux test binaries without executing hosted-runner KVM
  probes, while release CI owns real-KVM exercise.
- **Ordinary CI must not hide red signals.** Diagnostic-only steps should not
  use `continue-on-error`; make the diagnostic command itself non-fatal so a
  green job does not carry a red annotation. Test steps must not end in
  `|| true`, coverage summary pipes must use `set -o pipefail`, and Codecov
  test analytics should use `codecov/codecov-action@v5` with
  `report_type: test_results`.
- **No AppImage on any platform.** linuxdeploy cannot run on GitHub CI runners -- Ubuntu 24.04 lacks FUSE2, and neither `libfuse2` nor `APPIMAGE_EXTRACT_AND_RUN=1` fixes it reliably. All Linux platforms ship `.deb` only. CI matrix passes `bundles: deb` for both arm64 and x86_64. `just cross-compile` matches this. This cost 14 consecutive failed releases (v0.12.1 through v0.14.14) to discover.
- **Tauri signing keys on all platforms.** `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` must be passed to every `cargo tauri build` step (macOS and Linux). Missing keys cause "public key found but no private key" failure. The macOS job had them from the start; the Linux job was missing them until v0.14.11.
- **`just cross-compile` is not a perfect CI replica.** It runs in a Docker
  container on macOS and catches compile errors plus most `.deb` packaging
  issues, but environment differences can still slip through. Always verify the
  first CI run of a new Linux packaging change.
- **Platform-gate all macOS-only APIs.** Every use of `libc::clonefile`, `AppleVzHypervisor`, `core_foundation_sys`, etc. must be wrapped in `#[cfg(target_os = "macos")]` -- struct, impl, AND tests. The Linux app build compiles the full workspace. `cargo test --test platform_gating` catches ungated symbols at unit test time. This burned v0.14.7 through v0.14.9.
- **Pin Xcode version on macOS runners.** Always `sudo xcode-select -s /Applications/Xcode_16.2.app` (or latest) before any Apple toolchain use. GitHub periodically updates runner images and the default Xcode can break (Abort trap in xcodebuild). The preflight may pass on one runner instance while build-app-macos gets a different one. v0.14.12 failed because Xcode 15.4's xcodebuild crashed with `Abort trap: 6` when Tauri tried to locate notarytool -- despite zero workflow changes from v0.14.11 which passed 9 hours earlier.
- **Installer identity and Gatekeeper checks are release gates.** Release
  preflight must require `APPLE_INSTALLER_SIGNING_IDENTITY`, and it must start
  with `Developer ID Installer:`. Pass it into `scripts/build-pkg.sh` through
  the job environment, not inline expressions. After `xcrun stapler validate`,
  `build-app-macos` must run `pkgutil --check-signature` and
  `spctl -a -vv -t install` against the built `.pkg`. If a local macOS host
  reports Code Signing subsystem errors for multiple known-good releases, treat
  the host as suspect, but keep the CI macOS gate release-blocking.
- **Package metadata versions must match the release tag exactly.** The release
  validators compare `.deb` control metadata and `.pkg` distribution metadata
  to `GITHUB_REF_NAME#v`. Do not append a build timestamp in repackaging
  scripts; local install paths already stamp a fresh version before packaging
  when they need upgrade ordering. macOS `.pkg` manifest validation must also
  expand into a fresh directory or remove the previous expansion first.
- **`latest.json` is absent in the current release rail.** The current Linux
  rail is `.deb`-only and macOS ships a `.pkg`; there is no AppImage updater
  bundle. Do not make release creation depend on `latest.json`.
- **AppImage was dropped after 14 failed releases.** linuxdeploy (a FUSE2 AppImage) cannot run on Ubuntu 24.04 CI runners (FUSE3 only). Tested: `libfuse2` install, `APPIMAGE_EXTRACT_AND_RUN=1` env var, both together -- none worked reliably. If AppImage support is needed in the future, the approach would be to pre-extract linuxdeploy (`--appimage-extract`) and run the extracted binary directly, bypassing FUSE entirely.

## Full-test gates

| Gate | What |
|------|------|
| Unit tests | `cargo llvm-cov` with coverage |
| Cross-compile | capsem-agent for aarch64 + x86_64 musl |
| Frontend | `pnpm run check && pnpm run build` |
| capsem-doctor | Boot VM, run full diagnostic suite |
| Integration | Boot VM, exercise all 6 telemetry pipelines |
| Benchmark | Boot VM, run capsem-bench |

## Apple code signing

### p12 encryption (critical gotcha)

macOS Keychain only accepts legacy PKCS12 (3DES/SHA1). OpenSSL 3.x creates PBES2/AES-256-CBC by default, which Keychain rejects with "wrong password."

Check: `openssl pkcs12 -in cert.p12 -info -nokeys -nocerts -passin pass:PWD 2>&1 | head -5`
- `PBES2` = broken on macOS
- `pbeWithSHA1And3-KeyTripleDES-CBC` = works

Fix: `scripts/fix_p12_legacy.sh` then `gh secret set APPLE_CERTIFICATE < private/apple-certificate/capsem-b64.txt`

### Notarization

Shipping artifact on macOS is a **`.pkg`** (productbuild), not a `.dmg`. Flow:

1. `cargo tauri build --bundles app --skip-stapling` -- builds `.app` only (Tauri skips stapling the inner app; we staple the outer `.pkg`).
2. `scripts/build-pkg.sh` -- productbuilds `Capsem-$VERSION.pkg` with the `.app` + companion binaries + `manifest.json`. Heavy VM assets are downloaded on first use by the postinstall.
3. `xcrun notarytool submit ... --wait --timeout 30m` -- synchronous.
4. `xcrun stapler staple` + `xcrun stapler validate`.

Verify credentials locally (before touching a tag):
```bash
xcrun notarytool history --key private/apple-certificate/capsem.p8 --key-id KEY_ID --issuer ISSUER_ID
```

**403 "A required agreement is missing or has expired"** -- Apple periodically refreshes the Developer Program License Agreement, Paid Apps Agreement, etc. Only the **Account Holder** (not Admin/Developer) can accept. Check banners at both:
- https://developer.apple.com/account (Program License Agreement)
- https://appstoreconnect.apple.com → Agreements, Tax, and Banking (Free/Paid Apps)

Propagation can lag 1-5 min after accepting. `notarytool history` must return a list (possibly empty) before you tag -- the CI preflight step runs the same check and fails fast on 403.

When a release gate needs user action, say so plainly. Do not describe an
Apple agreement 403 as a transient notarization error or a credential problem.
Tell the user:

- what blocked the release (`notarytool history` returned 403 because an Apple
  agreement is missing or expired)
- why the agent cannot fix it (only the Apple Account Holder can accept it)
- exactly what to do (sign in to the two Apple pages above and accept any
  pending agreements or banking/tax terms)
- when to retry (after the agreement is accepted and 1-5 minutes have passed)
- what was intentionally not done (`just cut-release` was not run, so no
  release commit/tag/push happened)

If a timed retry is useful, offer or create a heartbeat retry. Keep the release
paused until `notarytool history` succeeds locally.

## CI secrets

| Secret | Purpose |
|--------|---------|
| `APPLE_CERTIFICATE` | Base64 `.p12` (legacy 3DES) |
| `APPLE_CERTIFICATE_PASSWORD` | Password for p12 |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Elie Bursztein (L8EGK4X86T)` |
| `APPLE_INSTALLER_SIGNING_IDENTITY` | `Developer ID Installer: Elie Bursztein (L8EGK4X86T)` |
| `APPLE_API_ISSUER` | App Store Connect issuer UUID |
| `APPLE_API_KEY` | App Store Connect key ID |
| `APPLE_API_KEY_PATH` | Contents of `.p8` private key |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater minisign key |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for Tauri key |
| `CODECOV_TOKEN` | Codecov upload token |
| `CLOUDFLARE_ACCOUNT_ID` | Cloudflare account that owns the Pages project serving `release.capsem.org` |
| `CLOUDFLARE_API_TOKEN` | API token allowed to deploy the Pages project serving `release.capsem.org` |

CI secrets are the source of truth for release signing. Local backups in
`private/apple-certificate/` and `private/tauri/` are useful for local preflight
and packaging checks, but they are gitignored and must never be staged.

### Release-channel Cloudflare prerequisites

Before running a live binary or VM asset channel deploy, create or verify the
Cloudflare Pages project serving `release.capsem.org`, attach the `release.capsem.org`
custom domain, and configure `CLOUDFLARE_ACCOUNT_ID` plus
`CLOUDFLARE_API_TOKEN` in GitHub Actions secrets. `release-channel.yaml` fails
before deploy if either secret is missing or
`scripts/check-cloudflare-pages-project.py` cannot see the Pages project through
the configured account/token, then runs `scripts/check-release-site-contract.py`
and smokes `https://release.capsem.org/`, `/channels.json`, and the channel
manifest through the public custom domain after Cloudflare publishes the
generated site. Live VM asset releases use the same project preflight before
the expensive asset build matrix starts.

## Post-release verification

```bash
gh release view vX.Y.Z
gh release download vX.Y.Z --pattern '*.pkg' -D /tmp/verify
pkgutil --check-signature /tmp/verify/Capsem-*.pkg
spctl -a -vv -t install /tmp/verify/Capsem-*.pkg      # Gatekeeper accepts notarized+stapled
xcrun stapler validate /tmp/verify/Capsem-*.pkg       # Staple ticket present
gh release download vX.Y.Z --pattern '*.deb' -D /tmp/verify
curl -fsSL https://release.capsem.org/channels.json -o /tmp/verify/channels.json
curl -fsSL https://release.capsem.org/assets/stable/manifest.json -o /tmp/verify/asset-manifest.json
uv run python3 scripts/check-public-binary-release.py \
  --channel stable \
  --manifest-url https://release.capsem.org/assets/stable/manifest.json \
  --install-script-url https://capsem.org/install.sh \
  --docker-linux-install \
  --docker-channel-switch \
  --docker-upgrade
```

Use `scripts/check-public-binary-release.py` for post-deploy glow-up instead of
ad hoc `tar`/`strings` checks. It validates public `install.sh`, package URLs,
package SHA-256, package-owned binary hashes, absence of packaged
`assets/manifest.json`, `manifest-origin.json` source provenance, Docker
install, stable/nightly asset switching, and the binary updater path. Package
scripts must not normalize or convert manifest JSON; the selected channel
manifest is the only runtime manifest format.

Binary GitHub releases publish host packages and the canonical host SBOM
artifact, `capsem-sbom.spdx.json`; the SBOM attestation subject list must cover
both `.pkg` and `.deb` package artifacts, and the release summary must say
`SBOM attested (SPDX 2.3, pkg + deb)`. VM asset manifests, blobs, OBOM evidence,
profile-owned records, channel manifests, and the root channel list live on
`release.capsem.org`; do not verify or publish VM `manifest.json` through the
tag release. Before recording binary metadata in the release channel, the tag
workflow preflights that the downloaded release artifacts contain
`capsem-sbom.spdx.json` and at least one installable host package (`.pkg` or
`.deb`).

Do not claim pre-updater installed clients can self-update just because the
release channel now advertises binary packages. Binaries that shipped before
the packaged binary updater must be manually bootstrapped once with the `.pkg`
or `.deb`; forward binary-update proof starts from an installed version that
already contains the updater and package apply path.

For a demo-facing macOS release, also prove the installer path users see:

```bash
just install
test -d /Applications/Capsem.app
open -a Capsem
pgrep -x capsem-service
pgrep -x capsem-tray
```

`scripts/build-pkg.sh` must install `/Applications/Capsem.app` and carry a
fallback app copy in `/usr/local/share/capsem/Capsem.app` so postinstall cannot
report success while the GUI is missing. Relaunching `Capsem.app` must ask the
running service to ensure the tray via `/companions/tray/ensure`; spawning
`capsem-tray` directly bypasses the service parent guard and is not the product
path.
## Documentation site

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

- **Binary**: `1.3.{unix_timestamp}` for the current release line -- auto-stamped by `just _stamp-version` on every `just install` and `just cut-release`. Set `CAPSEM_RELEASE_VERSION=x.y.z` when you need an exact preselected stamp.
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
