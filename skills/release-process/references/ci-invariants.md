# Release CI Invariants (hard-won lessons)

Reference for /release-process: the Ironbank parity rule and every burned-release lesson. Read before editing any release workflow.


#### Local/CI execution parity

##### Ironbank parity rule

The Ironbank parity rule is that every portable release gate must be owned by
`just test`. Before qualification, the exact candidate must pass the complete
`just test` locally; exact-SHA CI then runs that identical recipe. A specialized
job is useful evidence, but it must reuse an entrypoint already exercised by
`just test` and cannot become the sole owner of a portable release requirement.

The canonical gate includes workspace/runtime tests, Rust and Python coverage
floors, `capsem-doctor` and Ironbank acceptance, benchmarks, artifact
completeness, frontend/docs/marketing/release-site checks, and the
Docker/systemd Linux package install with a real guest-shell proof. Keep only
unavoidable platform boundaries outside it: Apple signing/notarization,
hosted-runner KVM, and Cloudflare publication. Apple VZ is owned by the complete
local gate on the exact clean candidate.

Every portable release-critical workflow must share the same production
entrypoint with a local gate. Current required mappings are:

- candidate qualification: local and CI both execute `just test`;
- VM assets: `just test` owns `just test-assets`, which executes the same
  `just build-kernel` and `just build-rootfs` primitives as
  `release-assets.yaml` for every checked-in profile and both published
  architectures, validates the full payload and manifest, and proves a real
  guest shell from each rebuilt host-architecture image;
- release-channel assembly: the local `release-site` gate and production asset
  and binary workflows execute `scripts/build-complete-release-channel.py`;
  every deployable production dist must contain and validate both `stable` and
  `nightly`, preserving the untouched channel graph instead of replacing the
  Pages site with only the channel being updated. Local qualification must
  materialize profile config from the same candidate worktree used to generate
  its descriptors, while production assembly must use an immutable git ref;
- VM asset digests: the asset build/ingest boundary streams every immutable
  blob once and persists BLAKE3 plus SHA-256 in the authoritative manifest.
  Channel assembly reuses those records and must not hash complete blobs once
  per channel or test fixture. Only a legacy current entry may be hydrated;
  historical releases never resolve through current asset paths. Local blob
  copying hashes while copying once, and graph rendering reuses that result;
- Linux package assembly: `just test` executes `just cross-compile arm64` and
  `just cross-compile x86_64`, so both publishable `.deb` architecture builds
  are locally accounted for before the release workflow repeats them;
- Linux package E2E: local and PR CI both execute `just test-install`;
- Linux platform branches: macOS-local `just test` executes
  `just test-linux-rust` in the checked-in host-builder container as a non-root
  user, while Linux CI calls the same runner natively;
- generated settings: `just test` regenerates the tracked settings outputs and
  fails if their before/after contents drift, matching CI's generation drift
  gate without requiring the local worktree to be committed first;
- package assembly and acceptance: local and release CI share
  `scripts/build-pkg.sh`, `scripts/repack-deb.sh`,
  `scripts/verify-installed-release.py`, and
  `scripts/prove-installed-shell.py`; macOS-local `just test` builds the real
  release-mode app and unsigned `.pkg`, both Linux release-mode `.deb`
  architectures, and runs `scripts/generate-host-binary-sbom.py` over those
  exact artifacts. `just install` must finish with the same installed-manifest
  verification and real guest-shell proof before success.

Run portable Linux prerequisites in Docker before spending CI. The container
must execute the same production entrypoint or shared predicate, not a copied
approximation. The native-musl regression is the model: asset CI installs
`musl-tools`, so both `just build-kernel`/`just build-rootfs` and the local
Docker preflight execute `linux_musl_toolchain_available`, which accepts the
native `musl-gcc` on arm64 and x86_64 without inventing an
`x86_64-linux-musl-gcc` requirement.

Release-only generators are immutable build inputs. Pin cdxgen to the same
exact version in the Python asset builder, `docker/Dockerfile.host-builder`, and
`release-assets.yaml`; contract tests must reject `@latest` and version drift.
VM OBOM generation targets the extracted guest filesystem directory, never the
build host `/`, using the invocation qualified against the complete Capsem
image. For cdxgen 12.7.0 that is `-t os`: its `rootfs` mode passes a minimal
fixture but emits invalid CycloneDX license data for the full image. A scanner
mode change therefore requires a full-image RED/green proof, not a toy fixture.
Successful scanner chatter must be captured/bounded, and failures must still
surface their diagnostics.

Keep post-processing tools native to the Docker host even when the artifact is
for another guest architecture. Every EROFS helper container must pass an
explicit host-native `--platform` (`linux/amd64` for x86_64/amd64 hosts and
`linux/arm64` for arm64/aarch64 hosts), reject unknown architectures, and
capture successful `mkfs.erofs` output. Without this, an arm64 asset build on
x86 CI can silently select arm64 `erofs-utils` and compress a multi-gigabyte
rootfs under QEMU; the corresponding Mac rail then has different behavior.
Treat runner loss during this rail as a release-blocking build-system bug
requiring a RED regression and a new exact candidate, not as a transient rerun.

The unavoidable platform boundaries are Apple signing and notarization,
hosted-runner KVM, and Cloudflare publication. The physical Mac runs the
complete local candidate gate before qualification, so VZ is not deferred to a
post-release proof. Keep each boundary's nearest deterministic contract, but
never claim the boundary itself from emulation: retain exact-SHA CI for hosted
services/KVM and the complete local VZ gate for the exact candidate. Any new
CI-only step must either gain a local shared-entrypoint proof or be added to
this explicit boundary list with its substitute and final authoritative gate.

Qualification must fit the runner that actually executes it with substantial
headroom for the final tests and evidence upload. Measure stage and total wall
times; do not infer capacity from `timeout-minutes`. Repeated termination at
nearly the same run age is a runtime-budget defect, not a transient runner loss.
Hold the release, reproduce the expensive rail through `just test` locally, and
shorten the critical path before dispatching again. Safe concurrency requires
per-lane workspaces, image tags, output directories, and cleanup ownership plus
a regression contract; two commands with different output arguments can still
race through a hidden fixed workspace.
The Docker daemon itself is shared across worktrees: automatic release gates
must never use `docker image prune --all`/`-a`, and parallel build primitives
must not invoke GC. A newly tagged cached image may retain an old creation
timestamp and be deleted by another lane's age-filtered prune. Cleanup belongs
at the outer owner and may prune only dangling images and unused builder cache;
captured daemon stderr must remain visible as release evidence.
Container-runtime preflights must prove bounded process completion, not merely
successful output. Never synchronize Colima's clock with a privileged Docker
`date -s` container: the command can print success and stop while the Docker
client hangs during cleanup. Both asset and package builds use the checked-in
host-side Colima clock synchronizer with a hard timeout and fail closed.

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
- **Three files hold the binary version.** `Cargo.toml` (workspace), `crates/capsem-app/tauri.conf.json`, `pyproject.toml`. `just _stamp-version` handles all three automatically. `just prepare-release` and `just install` call it; `just cut-release` must never stamp or commit.
- **Do not resurrect local VM manifest signing.** VM asset integrity is the
  profile manifest plus BLAKE3 hashes, manifest metadata/hash reporting, and
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

