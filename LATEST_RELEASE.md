version: 1.4.1782928920
---
### Fixed
- Fixed URL-backed release packages so remote asset manifests are fetched with
  the release validator user-agent and templated `{asset_version}` asset bases
  survive URL validation before VM asset hydration.
- Fixed macOS package assembly so the `Capsem.app` bundle version must match
  the package version before a `.pkg` can be built.
- Fixed live VM asset release discipline so `dry_run=false` preflights that the
  configured Cloudflare account/token can see the `release-eq7` Pages project
  before building VM images, publishing immutable asset blobs, or attesting them.
- Fixed release-channel deploy validation so Cloudflare publishes must run the
  Python release-site contract checker against `release.capsem.org`, validating
  content, evidence, hashes, attestations, and cache headers rather than only
  file presence.
- Fixed the release-channel reusable workflow contract so Cloudflare secrets are
  declared on `workflow_call` before the manual VM asset workflow invokes it.
- Fixed binary and VM asset release workflow permissions so callers grant
  `deployments: write` before invoking the reusable release-channel deploy.
- Fixed the VM asset release upload plan so dry-runs and live publishes include
  only real architecture artifacts, not `assets/current` compatibility aliases.
- Fixed the release-channel deploy workflow and release docs so
  `release.capsem.org` deploys to the actual Cloudflare Pages project
  `release-eq7`.
- Fixed the PR CI macOS non-VM integration lane so it codesigns the
  `capsem-bench-rs` binary produced by the `capsem-bench` package instead of a
  nonexistent `capsem-bench` executable.
- Fixed release-channel generation, validation, live smoke, and remote readiness
  so VM asset attestations must point at the published VM OBOM predicate
  evidence instead of passing with an omitted predicate URL.
- Fixed the release-process and asset-pipeline skills so they preserve the VM
  asset attestation predicate URL requirement for published VM OBOM evidence.
- Fixed the CI/release documentation so it preserves the VM asset attestation
  predicate URL requirement for published VM OBOM evidence.
- Fixed the asset-pipeline architecture documentation so it preserves the
  release-channel SBOM, VM OBOM, and VM asset attestation evidence contract.
- Fixed release-channel validation, live smoke, and remote readiness so host
  SBOM evidence must stay on the canonical `capsem-sbom.spdx.json` row and the
  host SBOM attestation must point at that evidence.
- Fixed release-channel validation, live smoke, and remote readiness so host
  SBOM and VM OBOM evidence must be valid SPDX 2.3 or CycloneDX documents, not
  just hash-matching files.
- Fixed release-channel validation, live smoke, and remote readiness so
  attestation evidence scope and workflow metadata cannot drift from the
  canonical host binary, host SBOM, or VM asset release rails.
- Fixed live release-site smoke/readiness validation so the immutable profile
  catalog artifact's state, current binary/assets targets, and compatibility
  fields cannot drift from `health.json` and the active channel manifest.
- Fixed live release-site smoke/readiness validation so the immutable profile
  catalog artifact is fetched and checked against its advertised BLAKE3 hash,
  schema, revision, and URL policy.
- Fixed live release-site smoke/readiness validation so current host binary
  package URLs, SHA-256 hashes, and sizes in `health.json` and release evidence
  cannot drift from the fetched channel manifest.
- Fixed live release-site smoke/readiness validation so current VM asset file
  URLs, BLAKE3 hashes, and sizes in `health.json` cannot drift from the fetched
  channel manifest.
- Fixed live release-site smoke/readiness validation so VM asset compatibility
  and newer-version flags cannot drift from the channel manifest's current
  asset release.
- Fixed live release-site smoke/readiness validation so profile catalog
  compatibility minima and newer-version flags cannot drift from the channel
  manifest.
- Fixed live release-site smoke/readiness validation so asset release history,
  deprecation state, deprecation dates, and minimum binary compatibility cannot
  drift from the channel manifest.
- Fixed live release-site smoke/readiness validation so stale `health.json`
  summary state cannot advertise the wrong channel, publication state, release
  URLs, or top-level binary/assets versions.
- Fixed live release-site smoke/readiness validation so image freshness remains
  explicitly unpublished until image release metadata is actually added to the
  asset channel.
- Fixed live release-site smoke/readiness validation so binary update target,
  state, source, and package file metadata cannot drift from the canonical
  tag-triggered binary metadata in `health.json`.
- Fixed live release-site smoke/readiness validation so VM asset update target,
  manifest, base URL, compatibility, and newer-version requirements cannot
  drift from the canonical asset channel metadata in `health.json`.
- Fixed live release-site smoke/readiness validation so profile update hash,
  compatibility, and newer-version requirements cannot drift from the canonical
  profile catalog metadata in `health.json`.
- Fixed live release-site smoke/readiness validation so
  `updates.profiles.source` must match the profile catalog source advertised to
  humans and machine readers.
- Fixed live release-site smoke/readiness validation so stale public index pages
  are rejected when profile catalog, generated timestamp, or channel manifest
  metadata drift from `health.json`.
- Fixed release-channel validation so a stale human `index.html` cannot pass
  while `/health.json` and the channel manifest advertise newer release state.
- Fixed the release-readiness docs and skills so the live activation order names
  the fail-closed remote `pr-gate` shape required before branch protection.
- Fixed the remote release readiness checker so it rejects `pr-gate` workflows
  that aggregate jobs but do not run fail-closed and assert every dependency
  result.
- Fixed the remote release readiness checker so malformed live release-site
  contract objects fail with explicit diagnostics instead of a Python exception.
- Fixed the release-process skill and Debian repack header so they document the
  `.deb` pre-install shutdown rail before binary replacement.
- Fixed the installation skill so it documents the Linux `.deb` pre-install
  restart rail that stops stale helpers before package replacement.
- Fixed Linux package self-updates so the repacked `.deb` carries a pre-install
  shutdown script for stale service, gateway, tray, and helper processes before
  binary replacement.
- Fixed the installation skill so it documents the full packaged host binary
  cohort and version-surface check used by installed update smokes.
- Fixed the remaining packaged admin and MCP helper binaries so they expose a
  package-version surface for installed update cohort verification.
- Fixed the install-layout test harness so it verifies every packaged host
  binary reports the installed Capsem package version.
- Fixed the gateway and tray helper binaries so they expose `--version` for
  installed update cohort verification.
- Fixed the service update routes so ambiguous JSON bodies with extra fields are
  rejected instead of silently checking or applying binary/profile or VM asset
  updates.
- Fixed CLI update-status formatting coverage so `capsem status` keeps
  available and blocked binary/profile/VM asset/image tracks separated.
- Fixed the binary release post-deploy smoke so asset-channel URLs must match
  the BLAKE3 hashes advertised by the public manifest, not just return HTTP 200.
- Fixed the VM asset workflow metadata preservation step so its writable local
  manifest path is not passed through the URL-only `--manifest` source flag.
- Fixed release-channel live smoke/readiness validation so host SBOM
  attestations must cover every published host package subject.
- Fixed release-channel validation so host SBOM attestations must cover every
  published host package subject.
- Fixed release-channel validation so host SBOM evidence must include matching
  host SBOM attestation metadata, not just package provenance.
- Fixed the tag-triggered binary release summary so it reports host SBOM
  attestation coverage for both `.pkg` and `.deb` package subjects.
- Fixed the tag-triggered binary release workflow so the canonical host SBOM
  attestation covers both macOS `.pkg` and Linux `.deb` package subjects.
- Fixed the tag-triggered binary release workflow so release-channel assembly
  preflights the canonical host SBOM and installable package artifacts before
  recording binary metadata.
- Fixed binary release metadata recording so only the canonical
  `capsem-sbom.spdx.json` artifact satisfies the host SBOM evidence
  requirement for the release channel.
- Fixed binary release metadata recording so package artifact filenames must
  match the binary version being advertised in the release channel.
- Fixed binary release metadata recording so zero-byte host package or SBOM
  artifacts are rejected before their hashes can be published to the release
  channel.
- Fixed binary release metadata recording so arbitrary non-SBOM files cannot
  satisfy the installable host package requirement; only `.pkg` and `.deb`
  artifacts count as host packages.
- Fixed binary release metadata recording so a release cannot update the
  release channel with only host SBOM evidence and no installable host package
  artifact.
- Fixed the manual VM asset release workflow so asset-channel builds preserve
  live binary release metadata, host SBOM references, and binary attestation
  state instead of replacing them with a freshly generated binary section.
- Fixed VM asset release delta checks so manifest policy changes such as
  `refresh_policy` deploy the release channel without republishing VM blobs.
- Fixed current VM asset release metadata comparisons so compatibility/date
  updates deploy the release channel without republishing unchanged VM blobs.
- Fixed the manual VM asset release delta so asset release metadata changes
  deploy the release channel without republishing unchanged immutable VM blobs.
- Matched attestation predicate URL validation to the correct evidence rail so
  VM asset provenance can point at VM OBOM evidence while host package
  attestations continue to point at host SBOM evidence.
- Rejected release-channel health indexes whose VM asset attestation predicate
  URL points outside the published VM OBOM evidence list.

### Changed
- Clarified that the first asset-channel bootstrap may omit host binary
  evidence until the tag-triggered binary rail publishes package files and host
  SBOM metadata, while later missing host SBOM evidence remains
  release-blocking.
- Removed the manual VM asset release `binary_version` override so asset
  releases preserve, but do not author, binary release metadata.
- Clarified VM asset no-op guidance so manifest policy changes such as
  `refresh_policy` deploy `release.capsem.org` without VM blob publication.
- Clarified VM asset release guidance so metadata-only asset channel changes
  deploy `release.capsem.org` without requiring immutable blob upload plans.
- Clarified manual VM asset no-op guidance so unchanged blob hashes alone do
  not imply a skipped release-channel deploy.
- Updated the remote release readiness checker to require the expanded
  `pr-gate` contract that includes docs and marketing builds.
- Moved docs and marketing PR build enforcement under the required `pr-gate`
  status while keeping the docs and marketing Cloudflare workflows as
  push-to-main deploy rails.
- Aligned self-update docs and skills with the implemented `capsem update --yes`
  behavior: verified `.pkg` and `.deb` installers are applied through the
  platform package manager rather than only printed.

### Added
- Added remote-readiness coverage for the first asset-channel bootstrap without
  host binary evidence while preserving the release-blocking host SBOM check
  once binary files are published.
- Added the live release activation order for publishing release-rail commits,
  enabling `pr-gate`, provisioning `release.capsem.org`, and sequencing binary,
  VM asset, and installed update smokes across release and asset-pipeline
  guidance.
- Added release-channel Cloudflare prerequisites for the `capsem-release` Pages
  project, `release.capsem.org` custom domain, and required deploy secrets
  across release and asset-pipeline guidance.
- Added asset-pipeline skill guidance that corporate VM asset channels use the
  same URL-only `--manifest` update contract as the public release channel.
- Added service-route regression coverage proving confirmed binary/profile
  updates and VM asset refreshes dispatch separate CLI commands.
- Added service and gateway update action routes that expose dry-run command
  plans for release checks, binary/profile updates, and VM asset refreshes
  while requiring explicit confirmation for live apply.
- Added profile-dashboard release-state rows for profile catalog, VM asset, and
  VM image freshness so new-session actions stay distinct from binary app
  updates.
- Added TUI update status and confirmed update actions for binary/profile
  updates and VM asset refreshes.
- Added compatible profile catalog application from the release channel so
  `capsem update` can refresh `~/.capsem/profiles` independently of binary and
  VM asset releases.
- Added canonical `release.capsem.org` endpoint URLs for the human index,
  health JSON, manifest, profile catalog, and asset base to the generated
  release-channel page and machine health index.
- Added asset release dates to the generated release-channel index and health
  JSON so `release.capsem.org` shows when each VM asset release was cut.
- Added release-channel deploy smoke coverage that rejects a public
  `release.capsem.org` index whose binary, VM asset, or asset date state is
  stale relative to the live health JSON and manifest.
- Added CI docs coverage noting that remote release readiness verifies live
  cache headers for mutable release-channel pointers and immutable artifacts.
- Added a dry-run `asset-release-plan` artifact for manual VM asset releases so
  reviewers can inspect the generated immutable GitHub Release upload commands.
- Added an `asset-release-delta` artifact for manual VM asset releases so
  changed and no-op dry runs preserve the manifest comparison decision.
- Added CI docs plus release-process and asset-pipeline skill guidance for
  dated asset release history and stale public index rejection.
- Added a read-only remote release readiness checker for `pr-gate`, branch
  protection/rulesets, DNS, and release-channel endpoint agreement.
- Added binary self-update package execution for verified macOS `.pkg` and
  Linux `.deb` installers when `capsem update --yes` finds a newer release.
- Added frontend release-channel evidence links for host SBOM, VM OBOM, and
  binary/VM asset attestations in the Settings/About update surface.
- Added VM asset provenance attestation references to the release-channel
  evidence index alongside OBOM and binary SBOM evidence.
- Added attestation predicate and verification command metadata to the
  release-channel evidence index so SBOM/OBOM/provenance checks are easier to
  audit from `release.capsem.org`.
- Added supply-chain evidence to update status responses, including manifest
  origin/hash, channel index hash, host SBOM, VM OBOM, and attestation
  references for UI, tray, TUI, and support-bundle consumers.
- Added explicit profile update semantics to profile list and status APIs so
  UI, tray, and TUI consumers know new sessions use the current profile catalog
  while existing VMs stay pinned until recreated.
- Published profile catalog revision, hash, source, and compatibility metadata
  in the release-channel index so profile freshness is no longer represented as
  an unpublished track.
- Stored release-channel cache provenance for update checks, including the
  fetched channel hash, validation status, and validation errors exposed through
  the update status API.
- Recorded package manifest snapshot provenance with fetched time, snapshot
  SHA-256, and package release version in installed manifest-origin metadata.
- Added install-side verification that `capsem update` fetches the release
  channel health index and writes validated channel cache provenance.
- Added install-side verification that corporate manifest origins derive their
  own release health endpoint while using the same update cache provenance as
  the public channel.
- Added install-side verification that profile catalog updates leave existing
  VM asset pin registries and installed asset manifests untouched.
- Added frontend update-status coverage for mixed binary and VM asset update
  summaries without treating binary releases as profile state.
- Added tray menu coverage for mixed binary and VM asset update state.
- Added TUI smoke-matrix coverage for mixed binary and VM asset update state.
- Added release-doctor coverage for the binary release package hydration smoke
  against public release.capsem.org asset URLs.
- Added release-doctor coverage that binary tag releases update release-channel
  metadata without rebuilding or publishing VM asset payloads.
- Added release-doctor coverage that package update scripts replace and restart
  the full app/service/gateway/tray helper cohort together.
- Added release-doctor coverage that prevents the binary release package
  hydration smoke from silently skipping missing `.deb` packages or bundled CLI
  binaries.
- Added release-doctor coverage that keeps staged cross-surface update smoke
  prerequisites visible across CLI, service, tray, frontend UI, and TUI tests.
- Added an installed CLI staged update-state matrix covering binary-only,
  asset-only, profile-only, and mixed binary plus VM asset release-channel
  states.
- Added CLI update unit coverage proving public and corporate update source
  flags share the same URL-only validation contract.
- Added service-route coverage proving corporate config source URLs reject bare
  filesystem paths across validate and edit endpoints.
- Added release-channel cache-header documentation coverage so CI docs,
  architecture docs, and agent skills stay aligned with the deploy smoke.
- Added release-doctor coverage that binary releases do not publish
  `latest.json` updater metadata and rely on release-channel health instead.
- Fixed the CI docs thin-package contract to state that installers carry host
  binaries and the selected manifest while VM assets stay remote.
- Proxied update status through the gateway so UI, tray, and TUI surfaces can
  read release-channel freshness state.
- Published profile catalog artifacts under the asset channel with portable
  release URLs so public profile metadata no longer exposes build-machine
  paths.

### Changed
- Changed docs and marketing workflows so every push to `main` redeploys and
  smokes the public sites while pull-request builds remain path-filtered.
- Changed release-process skill guidance to match the docs and marketing
  every-main-merge deploy rail.
- Changed docs and marketing site skills to preserve the every-main-merge
  deploy rail independently from binary and VM asset releases.
- Changed update status reporting so VM asset and image tracks can surface
  blocked release-channel candidates, including asset releases that require a
  newer Capsem binary.
- Changed profile catalog updates so `capsem update` reports available
  catalogs without applying them, while `capsem update --yes` performs the
  validated apply.
- Changed `capsem update` and `capsem update --check` output to report VM
  image track state separately and distinguish unknown installed VM assets from
  available VM asset updates.
- Guarded startup asset cleanup so deprecated VM asset releases cannot remove
  persistent VM boot asset pins while still allowing unpinned deprecated blobs
  to be cleaned up.
- Changed VM asset selection to skip deprecated asset releases for new
  sessions and asset hydration while showing deprecated releases in the
  generated release-channel history.
- Changed generated release-channel cache headers so mutable channel pointers
  remain no-cache while immutable asset and profile release artifacts are
  long-lived immutable.
- Replaced the placeholder frontend update helper with typed update check and
  apply actions backed by the service-owned release update routes.
- Changed update checks to use a non-mutating `capsem update --check` path so
  service/UI freshness probes refresh release-channel status without applying
  binary, profile, corporate config, or VM asset changes.
- Changed `capsem update` to reject `--assets --corp` so corporate VM asset
  channels flow through the same `--manifest <URL>` provenance path as public
  release-channel updates.
- Added release-index contract coverage proving manual VM asset releases update
  the asset channel without moving the current binary pointer.
- Added CLI integration coverage that keeps `capsem update --manifest` and
  `--corp` URL-only, while still allowing `file://` overrides for local
  corporate/update endpoints.
- Guarded binary release channel updates so profile catalog artifacts publish
  without rebuilding VM assets.
- Changed manual VM asset releases to publish GitHub build provenance
  attestations for kernel, initrd, rootfs, and OBOM artifacts.
- Changed manual VM asset releases to publish immutable `assets-v...` GitHub
  Releases with arch-prefixed VM blobs before deploying the channel.
- Updated binary tag releases to record package, SBOM, and attestation metadata
  in the release channel without rebuilding VM assets.
- Documented the PR `pr-gate` contract against `just test`, including the
  hosted-runner substitutions that must not be mistaken for full local release
  validation.
- Locked persistent VM asset-pin drift into the service route contract so
  profile or asset-channel updates cannot make an existing VM appear resumable
  with different boot assets.
- Exposed installed, latest, and blocked profile catalog freshness through the
  update status API so CLI, tray, UI, and TUI consumers can report profile
  updates without inspecting raw profile files.
- Changed `capsem update --assets` to refresh remote channel manifests from
  manifest provenance before downloading VM assets, so compatible asset
  releases can move independently of installed binaries.
- Changed the remote release-readiness checker to verify live
  `release.capsem.org` cache headers for mutable channel pointers and immutable
  asset/profile release artifacts.
- Changed the release-channel deploy smoke to verify public cache headers after
  Cloudflare publishes the generated site.

### Fixed
- Fixed developer setup docs and skills to call out `cdxgen` as a
  release-only prerequisite for local release workflow preflight.
- Fixed the release-channel deploy preflight to require generated Cloudflare
  cache headers before publishing `release.capsem.org`.
- Fixed the release-channel deploy preflight to require both the human index
  page and machine health JSON before publishing `release.capsem.org`.
- Fixed the remote release-readiness checker to report missing Python
  dependencies with an actionable `uv run` setup hint instead of a traceback.
- Fixed background release-channel refreshes so they honor the
  `app.auto_update` setting instead of checking daily after users disable
  automatic update checks.
- Fixed the Settings/About release-channel surface so blocked profile or asset
  tracks render as blocked instead of current.
