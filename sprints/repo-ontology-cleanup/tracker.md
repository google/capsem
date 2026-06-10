# Sprint: repo-ontology-cleanup

## Tasks

- [x] Create ontology sprint board.
- [x] Inventory top-level directories and tracked/generated split.
- [x] Initial root seed path corrected: no `config/guest`; profile-owned root
  lives under `config/profiles/<profile_id>/root/`.
- [x] Discovery: `guest/config` is mostly obsolete; it survives because
  `capsem-admin image build` still shells out to `capsem-builder build
  <guest_dir>`.
- [x] Discovery: Python builder abstraction is over-owning product config:
  scaffold/init/new/add commands, AI provider config, MCP config, web security,
  VM resources, VM environment, and defaults generation are all mixed into
  `GuestImageConfig`.
- [x] User contract: profile is the only ledger. Packages, MCP, root seed,
  assets, rules, plugins, and VM defaults must be in or referenced by the
  profile. If it is not in the profile, it does not exist.
- [x] User correction: there is no `config/guest`; it is `config/profiles`.
- [x] User correction: move `code.toml` into `config/profiles/code/profile.toml`
  so the profile directory is self-contained.
- [x] User correction: add profile-owned `mcp.json`, conventional package files
  for apt/Python/npm, a manual installer script, plus `tips.txt`.
- [x] User correction/security: profile must hash-pin referenced files, and
  package manifests must ship profile files with hashes.
- [x] User correction: Docker templates are config/build inputs and belong under
  `config/docker/`, not hidden in Python source.
- [x] Magic inventory pass found remaining suspicious paths:
  `guest/config`, builder templates under Python source, generated
  `config/defaults.json`/`settings-schema.json`/`mcp-tools.json`, root
  `.gemini`/`.claude`/`.agents`, old `CAPSEM_USER_CONFIG` and
  `CAPSEM_CORP_CONFIG`, `assets/current`, stale `rootfs.squashfs`,
  `sync-dev-assets.sh`/`simulate-install.sh`, and asset-only
  `manifest-origin.json`.
- [x] S0: Freeze current dirty install-log/version-stamp work.
- [x] S0: Add guardrail in active finalizing sprint.
- [ ] S1: Move host config source to `config/host/`.
- [ ] S1: Move Docker templates to `config/docker/`.
- [ ] S1: Move `config/profiles/code.toml` to
  `config/profiles/code/profile.toml`.
- [ ] S1: Define profile-owned package declarations for image-baked packages.
- [ ] S1: Define profile-owned MCP declarations.
- [ ] S1: Define profile-owned packaged root under
  `config/profiles/<profile_id>/root/`.
- [ ] S1: Define hash-pinned profile file references for enforcement,
  detection, MCP, packages, manual installer script, root, and tips.
- [ ] S1: Remove vague `guest_dir` as product config authority.
- [x] S1: Emit backend/CI build record with hashes for rendered Dockerfile,
  build context, rootfs tar, final EROFS, kernel assets, tool-version output,
  compression settings, git revision, and project version.
- [ ] S1: Extend build record to include profile and profile-owned payload
  files after the profile ledger hash schema lands.
- [x] Tooling: Add Ruff as a full-repository Python lint gate.
- [x] Tooling: Add `ty` as a Python source type-check gate for `src/capsem`.
- [ ] Tooling: Burn full-tree `ty` debt for guest payloads/scripts/tests after
  guest dependency paths and dynamic test helper types are normalized.
- [ ] S1: Delete/rewrite Python builder scaffolding and product config models.
- [ ] S1: Replace `GuestImageConfig` with backend-only image spec.
- [ ] S1: Remove settings/default generation from guest image config.
- [ ] S1: Resolve generated config files (`defaults.json`,
  `settings-schema.json`, `mcp-tools.json`) so they derive from host/profile
  truth or move under `target/config`.
- [ ] S1: Classify/remove root developer shims (`.gemini`, `.claude`,
  `.agents`).
- [ ] S1: Restrict or replace old config env overrides (`CAPSEM_USER_CONFIG`,
  `CAPSEM_CORP_CONFIG`).
- [ ] S1: Update code/tests/docs/skills; remove old-path fallbacks.
- [ ] S2: Add guest root seed and move AI config files into real files.
- [ ] S2: Add `mcp.json`, `apt-packages.txt`,
  `python-requirements.txt`, `npm-packages.txt`, `install.sh`, and `tips.txt`
  under `config/profiles/code/`.
- [ ] S2: Builder copies guest root seed into rootfs seed path.
- [ ] S2: `capsem-init` projects seed into runtime `/`.
- [ ] S3: Tool install refresh/version discipline.
- [ ] S4: Documentation and skill cleanup.
- [ ] S5: Verification gate.
- [ ] S5: Magic inventory gate.
- [ ] Changelog.
- [ ] Commit.

## Notes

- User-approved ontology: all configuration-shaped source belongs under
  `config/`. Host-side configuration should live under `config/host/`.
- Corrected guest root source:
  `config/profiles/<profile_id>/root/`.
- User direction: `guest/config` is probably 90% irrelevant now that
  `capsem-admin` owns image/profile/materialization. Do not preserve it by
  renaming everything blindly.
- User correction: there are no AI providers. MCP lives in profile or it does
  not exist. Packages baked into the image belong to the profile. Root seed
  files live under the profile, not `config/guest`.
- Security correction: path-only references such as `rule_files.enforcement =
  "profiles/code/enforcement.toml"` are not enough. The profile ledger must bind
  referenced files by blake3, and admin/doctor/service/package install must be
  able to verify/report that exact ledger.
- `config/profiles/<profile_id>/root/` represents guest `/`. Example:
  `config/profiles/code/root/root/.codex/config.toml` maps to
  `/root/.codex/config.toml`.
- Proposed target layout:
  `config/profiles/code/profile.toml`, `enforcement.toml`, `detection.yaml`,
  `mcp.json`, `apt-packages.txt`, `python-requirements.txt`,
  `npm-packages.txt`, `install.sh`, `tips.txt`, and `root/...`.
- `install.sh` is for profile-owned manual shell installers, for example AGY or
  Claude installer flows that cannot be expressed as apt/Python/npm package
  lines. It must be hash-pinned and audited as a build input.
- Runtime `/root` is tmpfs, so the root seed must be copied into a seed path in
  rootfs and projected at boot by `capsem-init`.
- Current `guest/config/ai/*.toml` has inline `files` entries for Codex,
  Claude, and Gemini, but those are not a trustworthy runtime projection path.
- `guest/config/` is widely referenced by builder, tests, docs, and skills; this
  is evidence of the old ontology, not proof that the whole shape should
  survive.
- Current admin path: `capsem-admin image build` validates the profile and then
  shells out to `uv run capsem-builder build <guest_dir>`. This is the seam to
  remove: admin should pass explicit image inputs to the backend.
- Python builder burn list:
  - delete `init`, `new`, `add ai-provider`, `add mcp` product-authoring CLI;
  - delete or rewrite `scaffold.py`;
  - remove `AiProviderConfig`, `McpServerConfig`, `WebSecurityConfig`,
    `VmResourcesConfig`, `VmEnvironmentConfig` from image build ownership;
  - keep only backend image concerns after admin/profile resolution: kernel arch
    config, resolved package install sets, rootfs compression, resolved root seed
    metadata, and version capture commands;
  - move/replace `generate_defaults_json` so host settings are generated from
    `config/host/settings.toml`, not guest image config.
- Dockerfile templates are not Python source. Move
  `src/capsem/builder/templates/Dockerfile.{rootfs,kernel}.j2` to
  `config/docker/` and make admin/backend hash them as build inputs.
- Current dirty worktree includes install timestamp log changes and version
  stamp files from the last successful `just install`. Freeze that before path
  moves.
- S0 freeze commit: `5d0bf0d4 fix: timestamp package install logs`.
- Build ledger first slice: `capsem-builder` now appends per-arch JSONL
  `build-ledger.log` from the production build path, and release CI uploads it
  as `vm-build-ledger-<arch>` even on failed builds. This is not the full
  profile payload hash contract yet; that remains open until `profile.toml`
  owns file hashes.
- Python tooling slice: Ruff is enabled for the full tree and has cleaned stale
  unused imports/dead assignments/undefined names. `ty check src/capsem` passes
  and is wired into CI/local gates. Full-tree `ty check .` still reports
  existing guest/test typing debt, mostly guest-only dependencies (`rich`,
  `fastmcp`, `capsem_bench` path setup) and dynamic tests; do not expand the
  gate until that debt is burned deliberately.

## Coverage Ledger

- Unit/contract: pending path resolver, profile file hash tests, MCP JSON parser
  tests, package file parser tests, and profile-root parser tests.
- Tooling: `uv run ruff check .` and `uv run ty check src/capsem` are the
  current Python quality gates.
- Functional: pending `capsem-admin image verify` and profile materialization.
- Auditability: backend build-ledger tests prove JSONL emission for rendered
  Dockerfile/build-context hashes, rootfs tar, EROFS, kernel assets, and tool
  versions. Pending: profile/payload hash records once profile hash schema
  lands.
- Adversarial: pending tests rejecting old paths/fallbacks, checked-in
  credentials in `config/profiles/<profile_id>/root/`, and mutated profile
  sibling files whose blake3 no longer matches.
- E2E/VM: pending `capsem-doctor` proof that seeded files exist in runtime
  `/root`.
- Telemetry: not directly touched unless doctor/status output changes.
- Performance: tool refresh may affect image build time; runtime should not add
  hot-path latency.
- Missing/deferred: none accepted yet.
