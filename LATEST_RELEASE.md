version: 1.5.1784475356
---
### Fixed
- Isolated the packaged Linux install and channel glow-up harness from ignored
  developer VM assets, forwarding the selected fixture tree through every
  repack and channel stage so local runs cannot copy multi-gigabyte root files
  into Docker temp storage while clean CI uses tiny fixtures.
- Made the install/release test-asset generator emit the same rootfs-scoped
  CycloneDX contract as production and replace stale live-host inventories,
  preventing dirty developer assets and clean CI fixtures from diverging at
  the public release-channel validator.
- Replaced cdxgen's live-host `os` inventory with a deterministic scan of the
  exported Debian guest rootfs, normalized the pinned scanner's invalid
  lowercase Sendmail license and colliding certificate subset before strict
  schema validation, and made local/public release gates reject unscoped or
  osquery-backed host OBOMs. Added an exact-SHA macOS/Linux GitHub
  materialization preflight so Cloudflare client-policy or manifest-schema
  failures stop before the multi-hour qualification gate.
- Hardened release manifest consumers against Cloudflare rejecting Python's
  default HTTP identity, taught package materialization and asset-delta checks
  to distinguish the public release graph from the legacy VM asset manifest,
  replaced the post-deploy legacy-only URL verifier with a byte/BLAKE3 checked
  dual-schema rail, and made release preparation stamp before the full local
  package gate so both schemas and the exact candidate version are exercised
  before qualification.
- Made VM asset generation stream BLAKE3 and SHA-256 together into the
  authoritative manifest, made channel assembly reuse those digests instead
  of repeatedly reopening gigabyte root files, isolated historical releases
  from current asset paths, and replaced release graph tests' real asset tree
  with deterministic prepared fixtures while retaining fail-closed local-copy
  mutation coverage.
- Pinned cdxgen 12.7.0 across local and CI asset rails, captured successful
  scanner chatter, and forced EROFS helper containers onto the Docker host's
  native Linux platform on Intel Linux and Apple Silicon macOS so cross-guest
  builds cannot silently compress multi-gigabyte images under QEMU or overwhelm
  exact-SHA qualification with per-file output.
- Made local release glow-up staging hardlink immutable package and VM blobs
  on the same filesystem on macOS and Linux, with a tested cross-filesystem
  copy fallback, so exact-SHA qualification cannot exhaust runner disk by
  duplicating the multi-gigabyte asset cohort at the final install gate.
- Made the fail-fast hardcoded release-selection guard run with Python's
  standard library alone, so a clean qualification runner without `rg` cannot
  burn a complete candidate gate before testing begins.
- Removed the unused non-crossable librsvg introspection toolchain from Linux
  package cross-builds, and pinned both VM profiles to checksum-verified
  Antigravity CLI release artifacts after the vendor installer URL disappeared.
- Replaced privileged Docker clock mutation with a bounded Colima VM clock
  synchronizer, preventing both silent apt date skew and Docker clients hanging
  after the clock-setting container exits.
- Made local release-site graph qualification materialize profile files from
  the candidate worktree instead of the previous `HEAD`, while production
  release assembly retains immutable git-ref sourcing.
- Provisioned and preflighted `zstd` in local macOS bootstrap/doctor and the
  exact-SHA Linux qualification gate, and made host-SBOM generation fail with
  a direct prerequisite error before invoking `tar`. This closes the parity
  hole where `just test` could build every release package before discovering
  that macOS could not inspect the Linux `data.tar.zst` payloads.
- Replaced installed-update tests' retired release endpoint environment
  overrides with package-shaped `manifest-metadata.json` provenance, and made
  the hardcoded-selection guard reject either override if it returns to the
  installed test suite.
- Masked `systemd-binfmt` in privileged Linux install-test containers and made
  the local install gate prove Colima's live Rosetta registration survives the
  full systemd/package/glow-up lifecycle. The container previously flushed the
  host VM's binfmt table, making the later doctor/recipe rail fail after the
  install rail had otherwise passed.
- Made macOS bootstrap wait for registry DNS inside a real container after a
  Colima restart. A cached `alpine true` probe previously reported success
  while the VM DNS forwarder was still starting, allowing the next package
  image build to fail resolving GHCR.
- Made release qualification validate the live public manifest with the exact
  candidate runtime, and made update checks reject profile graphs that the
  runtime cannot install, including graphs missing required image revisions.
- Made the Linux doctor accept the portable native `musl-gcc` supplied by the
  supported packages instead of requiring an x86-only cross-compiler on arm64
  asset-build runners, and exercise the same check in the local Docker
  preflight before expensive release gates.
- Made local/CI release-path parity a tested project rule, documented the
  native musl doctor miss as a hard-won lesson, and required explicit
  authoritative gates for platform boundaries that cannot run locally. The
  local Linux release container now also carries and preflights the same
  `cdxgen` asset-evidence tool installed by asset CI, refreshes its base image
  from the checked-in Dockerfile, and makes `just install` verify the installed
  manifest and execute a real guest-shell marker before succeeding.
- Made the Ironbank parity rule explicit: every portable release gate is owned
  by `just test`, which now validates the frontend, docs, marketing site, and
  generated release-site channel through the same checked-in entrypoint used
  by CI and release workflows.
- Made macOS-local `just test` exercise Linux-only Rust branches in a faithful
  non-root Docker environment, fail generated-settings drift, build the real
  release-mode macOS package and both Linux packages, generate the production
  host SBOM from those exact artifacts, and execute the previously omitted
  recipe tests after VM proofs complete.
- Made exact-SHA Linux qualification require KVM for the native package without
  rejecting the structurally validated non-host package before the native
  systemd and guest-shell proof can run.
- Made parallel VM asset qualification preflight Docker daemon capacity,
  reclaim unused builder cache before extraction, and retain fully flushed,
  architecture-specific failure logs.
- Removed silent `code` selection from desktop shortcuts, the create dialog,
  the tray, CLI run/MCP commands, and MCP tools. Profile-scoped operations now
  carry an explicit or catalog-selected profile, and the UI fails closed when
  the installed profile catalog cannot be loaded.
- Made release packages materialize and validate every installed profile,
  bound exact-SHA qualification to the exact stable/nightly channel, and added
  a `just test` grep guard for current and planned profile names, channel URLs,
  package rails, and qualification calls.
- Removed the native macOS/Linux postinstall fallback to the stable manifest;
  installers now require the package-generated `manifest-metadata.json` and
  its manifest URL instead of silently changing channel when metadata is
  missing.
- Made isolated `CAPSEM_HOME` service launches bind an ephemeral gateway port,
  and made `capsem shell` wait for and pass that exact runtime endpoint to the
  TUI instead of silently falling back to another installation on port 19222.
