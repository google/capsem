version: 1.5.1783439394
---
### Fixed
- Split the binary release rail across stable and nightly manifests for the
  first 1.5 publish, proving package/SBOM metadata can move without rebuilding
  or mutating VM image/profile metadata.
- Added a manifest history audit regression proving retained channel manifest
  records use status enums, full digests, and never publish `removed`.
- Added a public release HMAC regression proving generated release pages and
  JSON graph inventory expose only SHA-256 and BLAKE3 digest fields.
- Added a release-site canonical URL regression proving human pages expose
  `/assets/<channel>/manifest.json` and not versioned manifest fetch endpoints.
- Added a release-site generation regression proving profile catalog side
  channels stay out of the public pages and graph.
- Added an adversarial software inventory regression proving distinct package
  rows cannot share identical SHA-256/BLAKE3 digest pairs.
- Added a software inventory hash regression proving each row owns distinct
  SHA-256 and BLAKE3 values instead of reusing inventory evidence digests.
- Added a software inventory version regression proving profile package rows
  reject missing, unknown, unversioned, and fallback versions.
- Added a software inventory evidence regression proving each architecture
  links its inventory once and keeps row hashes row-owned.
- Added a profile page architecture regression proving package, config, image,
  and evidence records stay in their owning architecture blocks.
- Added a profile software architecture regression proving software inventory
  rows live under architecture nodes and never render as architecture `all`.
- Added a binary description regression proving executable descriptions come
  from generated metadata and never from release-site fallback text.
- Added a macOS package cohort regression proving the package owns app, tray,
  helper, gateway, service, MCP, TUI, admin, and CLI executable inventory rows.
- Added a binary SBOM reference regression proving binary rows keep component
  refs and do not repeat package SBOM file links.
- Added a package SBOM owner-level regression proving SBOM links render once
  on the owning package page and not on each binary row.
- Added a package detail navigation regression proving every manifest package
  has a generated package page linked from its channel page.
- Added a channel package table regression proving package rows render under
  explicit OS/architecture sections with no architecture-all fallback.
- Added a package-target regression proving channel manifests and pages group
  Capsem packages by explicit operating system and architecture JSON fields.
- Added a manifest-version independence regression proving channel manifest
  versions stay separate from binary package and profile asset versions.
- Added a root channel metadata regression proving the table uses manifest
  version, last updated, and manifest URL labels without selected/status/records jargon.
- Added a root channel page regression proving stable/nightly rows render
  descriptions once and do not repeat raw channel identifiers.
- Added a human hash display regression proving release-site pages truncate
  SHA-256 and BLAKE3 values to 8-character prefixes while machine JSON stays full.
- Added a named Codecov binary/release-target regression proving CI covers
  executable targets, release-critical components, and package rail tests.
- Added a named Codecov crate reporting regression proving every Rust
  workspace crate source directory appears in the uploaded coverage surface.
- Added a named profile evidence scoping regression proving ABOM and OBOM
  evidence stays under profile image artifacts, not loose channel/profile evidence.
- Added a named profile image bundle regression proving each profile
  architecture renders the complete kernel, initrd, and rootfs artifact set.
- Added a named profile config inventory regression proving security,
  detection, MCP, package-manager, build, tips, and root manifest config files render.
- Added a named software-inventory real-version regression proving profile
  software rows reject missing, latest, unknown, and unversioned placeholders.
- Added a named macOS package binary cohort regression covering app, tray,
  helper, CLI, hashes, installed paths, and SBOM component refs.
- Added a named package-owned-binaries regression so package detail pages list
  only the executable files inside the selected package target.
- Added a package SBOM owner-level regression proving package pages keep SBOM
  evidence in package metadata instead of repeating it on binary rows.
- Added a named channel package grouping regression proving package rows stay
  under OS/architecture target sections on channel pages.
- Added a manifest package-target gate proving package and binary rows carry
  explicit operating-system and architecture coordinates in machine JSON.
- Added a manifest-version independence gate so channel manifest versions stay
  separate from Capsem package, profile, and VM asset/image versions.
- Added a named root-channel last-updated regression so channel rows keep
  manifest revision/update metadata and reject status/records columns.
- Added a named root-channel duplicate-id regression so stable/nightly rows
  render metadata descriptions instead of repeating raw channel identifiers.
- Fixed the local release-site contract checker so filesystem release outputs
  validate stable and nightly channel pages without stale asset-version or
  production-only HTTP header assumptions.
- Added an ABOM/OBOM image-scope gate so profile evidence must point at the
  owning architecture path and cannot leak into channel or generic evidence tables.
- Added a profile-image completeness gate so every profile architecture must
  publish kernel, initrd, and rootfs artifacts before release validation passes.
- Added a profile software inventory regression proving software rows are
  scoped to architecture blocks and never flattened as architecture `all`.
- Added explicit release-site labels for profile config enum values so rendered
  config tables no longer pass arbitrary kind strings through as UI labels.
- Added a profile config inventory regression proving every architecture renders
  MCP, enforcement, detection, package-manager, build, tips, and root manifest files.
- Added a byte-backed package/binary hash regression covering package SBOM
  artifact digests and binary SHA-256 component checksums from SPDX evidence.
- Added a SHA1-only SBOM regression proving SPDX file evidence must include
  SHA-256 checksums; SHA1 is ignored as upstream noise, not trusted evidence.
- Added a Linux package cohort regression proving each Debian architecture
  owns the full Capsem binary set with paths, digests, and SBOM refs.
- Added a macOS package cohort regression proving app, tray, CLI, service,
  gateway, MCP, and TUI binaries are package-owned with paths, digests, and SBOM refs.
- Added a package-detail SBOM regression proving package evidence is rendered
  once while binary rows show only their SBOM component references.
- Added a package-architecture regression proving channel package grouping uses
  explicit JSON platform/architecture fields even when the package filename lies.
- Added a profile/image revision SemVer gate and admin validation so profile
  payload versions are channel-scoped release coordinates, not date strings.
- Added a profile-image removal gate proving image removal is represented by
  omission from the current profile and `status: removed` is rejected.
- Added a nightly binary/package isolation gate proving binary updates do not
  mutate profile, stable, or channel state.
- Added a co-work nightly profile isolation gate proving profile updates do not
  mutate stable, code, package, binary, or sibling architecture state.
- Added a stable-to-nightly manifest URL switch gate and channel identity in
  graph manifests so clients can validate selected channel state without a side registry.
- Added an immutable manifest-history gate proving old channel manifest records
  stay addressable while current uses the canonical asset URL.
- Hardened the release-graph status enum gate so `removed`, `payload_status`,
  and deprecated boolean fields cannot survive in generated graph JSON.
- Hardened the root channel update-time gate so channel rows show generated
  provenance and never reintroduce Status/Records noise.
- Hardened the root channel-description review gate so stable/nightly copy is
  rendered from channel metadata in the matching channel row.
- Added a release-site content validator regression proving remote deploy checks
  package-detail and profile-artifact page values, not just file existence.
- Added a named Codecov crate-and-binary enumeration gate covering workspace
  crates, binary targets, Codecov components, `--bins`, and package rail tests.
- Added a generated-from-JSON release-site gate proving root, channel, package,
  binary, profile, software, config, image, and evidence cells render owner JSON.
- Added a named placeholder-or-copied-hash gate covering fake digest patterns,
  copied row digests, and duplicate digest reuse across release artifacts.
- Added a full-site digest display regression so human release pages must show
  SHA-256 and BLAKE3 values as short labels without leaking full machine hashes.
- Hardened the ABOM/OBOM architecture-scope gate so profile pages render image
  evidence in the image-owned block, not generic profile evidence.
- Hardened the profile image artifact gate so every architecture-local image
  block renders each image artifact's kind, name, URL, and digests.
- Added a named all-config-classes gate covering rendered profile config
  classes and source TOML-owned security, detection, and MCP config paths.
- Added a named real-software-versions gate covering forbidden software
  versions, row-owned hashes, and inventory-digest reuse rejection.
- Added a named review gate proving software inventory evidence appears once
  per profile architecture rather than repeating on every software row.
- Hardened the software architecture scope gate so rendered profile software
  sections cannot show `all` instead of concrete profile architectures.
- Hardened the profile architecture section gate so every current stable and
  nightly profile page must render profile-owned architecture sections.
- Hardened the binary-description release gate so every package page renders
  source-owned binary descriptions for both stable and nightly packages.
- Added a named owned-binary-cohort gate proving package detail pages list all
  binaries owned by each package while channel pages stay package-focused.
- Added a package-target parity gate requiring current channel manifests and
  channel pages to include macOS arm64 and Linux arm64/x86_64 packages.
- Added a named package-target SBOM regression gate so every package row must
  expose the SBOM evidence owned by that package target.
- Added a release-site review regression that rejects profile catalog files
  and profile-catalog page links as a second profile registry.
- Added a release-site review regression that rejects versioned manifest paths
  and profile-catalog links as alternate client entrypoints.
- Labeled channel-page manifest versions explicitly so package versions,
  profile revisions, and manifest versions stay visually separated.
- Added a release-site review regression gate for root channel table semantics
  so Selected/Status/Records labels cannot return.
- Removed hardcoded release-site channel description fallbacks so root channel
  descriptions are rendered only from channel metadata.
- Added a binary-target coverage workflow gate that derives executable-owning
  crates from Cargo metadata and verifies CI coverage includes them.
- Added an adversarial Codecov component contract that fails when a workspace
  crate path disappears from coverage reporting.
- Appended Python release/package integration tests to the uploaded coverage
  report so package rail coverage is visible in Codecov.
- Added release-site coverage generation and Codecov upload wiring so the
  Astro release channel surface is visible in PR coverage reports.
- Required Rust coverage workflows and local coverage recipes to include
  binary targets with `--bins`.
- Split low-coverage MCP aggregator, MCP builtin, process, and mock-server
  crates into dedicated Codecov components with explicit project targets.
- Added a named Codecov component contract that fails when any Cargo workspace
  crate path is missing from `codecov.yml`.
- Added a top-level CI coverage contract that enumerates Cargo workspace crates
  and release binary targets from `cargo metadata`, then verifies macOS
  coverage commands include every owning package.
- Added named root channel metadata gates for generated and HTML release-site
  output, and moved stable/nightly descriptions into the release graph fixture
  so the root page text is JSON-owned.
- Added a named root release-channel metadata gate proving the index renders
  manifest revision, update time, package/profile coverage, architectures, and
  canonical manifest URLs without Selected/Status/Records noise.
- Hardened the release-site readiness checker so stale generated HTML is
  rejected when channel pages stop matching the source channel, manifest,
  package, profile, and package-owned binary JSON values.
- Added a release-site graph mutation gate proving Astro pages render channel,
  package, binary, profile, software, and image fields from the JSON graph, and
  made the local `build:channel` gate build against the checked-in fixture when
  no release-channel output path is provided.
- Added a named release-site gate proving generated pages and release-site
  loaders do not reintroduce a profile catalog side channel.
- Added an explicit release manifest-version rail so channel manifests publish
  `version` values independently from Capsem package versions, VM asset
  versions, profile revisions, and profile image revisions.
- Added an independent release-version matrix gate and explicit profile
  architecture package-inventory and image revision fields so manifest,
  package, profile, software-inventory, and image versions can move separately.
- Fixed the release-channel deploy validation rail so Cloudflare checks stable
  and nightly content through the public domain, preserves package metadata
  from graph manifests, accepts generated profile config enums, and validates
  package-owned SBOM evidence.
- Added a stable/nightly switch gate proving canonical channel manifest URLs
  resolve distinct package and profile state without cross-channel contamination.
- Added a nightly co-work profile isolation gate proving a profile architecture
  update does not mutate stable, code, sibling architectures, packages, or binaries.
- Added a release-lane independence gate proving profile updates mutate only
  the selected profile payload while package and binary inventories stay fixed.
- Added a release-lane independence gate proving binary/package updates mutate
  package-owned data without changing profile payloads or other channels.
- Added a named copied-inventory digest gate proving software rows do not reuse
  software-inventory evidence hashes.
- Added a named release-site digest display gate proving human pages truncate
  hashes while machine JSON keeps full SHA-256/BLAKE3 values.
- Repaired the stable/nightly release graph fixture so shared software
  inventory evidence uses canonical asset URLs and distinct rendered inventory
  rows cannot reuse digests for different subjects.
- Rejected repeated release-graph digests across distinct software rows and
  profile artifact entries so copied hash evidence cannot pass validation.
- Rejected placeholder release digests in readiness validation so repeated
  one-character SHA-256/BLAKE3 values cannot pass as release evidence.
- Rebuilt the release-channel contract around generated channel, manifest,
  package, binary, profile, image, and evidence ownership; added multichannel
  stable/nightly generation and live `release.capsem.org` verification gates
  with real SHA-256 and BLAKE3 artifact checks.
- Hardened the generated release-site display contract so root/channel pages use
  current manifest language, hide legacy profile-catalog side links, truncate
  human-facing hashes, keep SBOM evidence on package rows, and require packages
  to own executable binary inventory in release-channel tests.
- Added named release-site HTML gate tests for Sprinty closure and serialized
  Astro fixture builds so concurrent release-site contract checks cannot corrupt
  `release-site/dist`.
- Added named release architecture and package/binary contract gates for the
  canonical channel manifest URL, package-owned executable inventory, and
  package-level SBOM evidence.
- Documented and gated the release graph invariant hierarchy from channels to
  packages/binaries and profiles/architecture-scoped payloads.
- Documented and gated independent release version surfaces so package,
  profile, and profile-image updates can move without mutating each other.
- Documented and gated the single release status enum across graph status
  fields, with no `removed` status.
- Added a release hash/evidence gate that rejects HMAC fields in the public
  release graph.
- Added a generated release-site ownership gate so root, channel, and profile
  pages cannot display facts owned by a different JSON object.
- Grouped release-channel package tables by package architecture on the
  generated channel pages.
- Added a release package gate that requires the macOS `.pkg` package to appear
  in the package inventory and generated channel page.
- Added a package-owned binary cohort gate requiring each release package to
  list the expected Capsem executables.
- Split the root release channel page from host package versions by rendering
  independent manifest revisions from `channels.json`.
- Replaced root release-channel history/record noise with generated update and
  package/profile/architecture coverage metadata.
- Split generated release-channel package tables by explicit architecture and
  platform target so host packages are not grouped into ambiguous architecture
  buckets.
- Required package-scoped SBOM evidence for each generated release package
  instead of cloning the global host SBOM into every package row.
- Made generated package detail pages identify the selected package and guard
  against leaking sibling package binaries or evidence.
- Moved release-channel binary descriptions into generated package metadata and
  required the release site to render those descriptions from machine JSON.
- Rendered package-owned binary targets from the owning package architecture and
  platform instead of hiding target information from human release pages.
- Added a named package-detail rendering gate proving package SBOM evidence is
  shown once at the package level and not repeated on binary rows.
- Added a named package SBOM contract gate so Sprinty verifies per-target
  package SBOM evidence instead of relying on broad test selection.
- Removed the release profile page fallback to binary compatibility metadata so
  profiles expose only profile-owned minimum Capsem requirements.
- Moved generated profile release payloads under architecture-owned software,
  config, image, and evidence blocks, and updated release-site validation so
  human pages keep one canonical manifest URL while machine catalogs retain
  immutable audit records.
- Rendered release-channel package groups as explicit OS/architecture package
  targets and added Linux amd64 fixture coverage alongside macOS arm64 and
  Linux arm64 packages.
- Required every release package to generate a detail page with the package
  target, hashes, SBOM evidence, and owned binary inventory rendered from the
  machine graph.
- Expanded release-package binary inventory fixtures and gates so every package
  lists the full Capsem executable cohort, including app, tray, CLI, service,
  process, TUI, MCP, gateway, and admin binaries.
- Kept package SBOM evidence on the package detail evidence block, with binary
  rows limited to SBOM component references instead of repeating evidence URLs.
- Added package SBOM fixture files and gates that prove every package-owned
  binary SBOM component reference resolves inside the owning package SBOM with
  SHA-256 checksums matching the binary inventory.
- Required channel package target rows to show each target package SBOM link,
  byte count, SHA-256, and BLAKE3 evidence instead of a partial shared digest.
- Removed the flattened channel-level binary table so executable inventory is
  rendered only on the owning package detail pages.
- Corrected profile architecture evidence so ABOM, OBOM, and software
  inventory are scoped to the profile architecture that owns them.
- Required profile software inventory rows to carry real package versions,
  rejecting placeholder values in generated graph validation and remote
  release-site readiness checks.
- Added the named Sprinty gate for real profile software versions so empty,
  `unversioned`, `unknown`, and `latest` rows fail release readiness checks.
- Added named release-contract gates proving profile lists come from channel
  manifests without a detached profile catalog side channel.
- Moved profile ABOM, OBOM, and software-inventory evidence links to the top of
  each profile architecture block and stopped repeating evidence URLs on
  software rows.
- Normalized release package architecture metadata to the profile graph's
  canonical `x86_64` spelling so package availability joins by channel
  architecture instead of profile-owned package theater.
- Added a named software-evidence scope gate proving profile software rows do
  not repeat architecture-level evidence URLs.
- Added named profile and hash-evidence gates proving each architecture renders
  one software-inventory evidence link while software rows keep row-owned
  digests.
- Required profile software row digests to be derived from the row's
  name/version/source/architecture/evidence tuple instead of copied inventory
  or placeholder hashes.
- Rejected profile software rows that copy the architecture software-inventory
  file digest in both release graph validation and remote release readiness.
- Attached profile ABOM and OBOM links to the profile image block for each
  architecture while keeping software inventory evidence at the architecture
  evidence header.
- Added a named profile image evidence gate proving ABOM and OBOM links stay
  in the owning architecture image block and do not leak onto channel pages.
- Renamed profile architecture software tables to installed software and added
  a gate proving channel package names and URLs do not leak into profile
  installed-inventory blocks.
- Added a named profile page section-boundary gate proving installed software,
  config files, image artifacts, and image evidence render in separate
  architecture-owned blocks.
- Added a named profile manifest-shape gate proving architecture software,
  config, image, and evidence lists stay distinct without an ambiguous
  profile-owned package bucket.
- Added a named profile config-class gate proving generated profile pages
  render profile, MCP, enforcement, detection, package-manager, build, tips,
  and root-manifest config entries.
- Added a named config inventory gate proving release profile config artifacts
  match canonical profile TOML file definitions, including enforcement and
  detection rule files.
- Updated asset-channel validation to require OS/architecture package target
  sections and package detail links instead of the removed flattened channel
  binary table, unblocking generated release-output contract checks.
- Replaced profile config kind strings with a release graph enum and updated
  remote readiness validation to reject unknown config categories.
- Added a full-machine-digest gate proving release graph JSON keeps complete
  SHA-256 and BLAKE3 digests for manifests, packages, binaries, profile
  config, images, software rows, ABOM, OBOM, and software inventory evidence.
