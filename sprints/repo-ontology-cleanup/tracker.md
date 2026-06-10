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
  `.gemini`/`.claude`/`.codex`, old `CAPSEM_USER_CONFIG` and
  `CAPSEM_CORP_CONFIG`, `assets/current`, stale `rootfs.squashfs`,
  `sync-dev-assets.sh`/`simulate-install.sh`, and asset-only
  `manifest-origin.json`.
- [x] S0: Freeze current dirty install-log/version-stamp work.
- [x] S0: Add guardrail in active finalizing sprint.
- [ ] S1: Move host config source to `config/host/`.
- [x] S1: Move Docker templates to `config/docker/`.
- [x] S1: Move `config/profiles/code.toml` to
  `config/profiles/code/profile.toml`.
- [x] S1: Define profile-owned package declarations for image-baked packages.
- [x] S1: Define profile-owned MCP declarations.
- [x] S1: Define profile-owned packaged root under
  `config/profiles/<profile_id>/root/`.
- [x] S1: Define hash-pinned profile file references for enforcement,
  detection, MCP, packages, manual installer script, root, and tips.
- [ ] S1: Remove vague `guest_dir` as product config authority.
  Partial: `capsem-admin image build` now materializes
  `target/image-workspace/<profile_id>/guest` from the profile before invoking
  the backend, but the Python backend still accepts a guest directory and must
  be demoted to an explicit image spec in a later slice.
- [x] S1: Emit backend/CI build record with hashes for rendered Dockerfile,
  build context, rootfs tar, final EROFS, kernel assets, tool-version output,
  compression settings, git revision, and project version.
- [x] S1/S5: Restore Linux KVM guest-memory safety hardening from lost Linux
  line:
  `0422a6ec` guest memory range validation and `45800223` offset-overflow
  guards are ported into current KVM memory/virtio-blk code.
- [ ] S5: Boot rebuilt profile and run AGY/Antigravity in the guest. Do not
  raise VM RAM caps speculatively; capture the exact kernel/runtime failure and
  fix the specific kernel option if it still fails.
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
- [x] S1: Classify/remove root developer shims (`.gemini`, `.claude`,
  `.codex`): `config/skills/` is the only skill source; root skill symlinks
  are removed; profile/agent injection must copy or mount from
  `config/skills/` explicitly.
- [x] S1: Add Pydantic-backed skill library validation. `capsem-builder
  validate-skills config/skills` validates every skill directory and
  `SKILL.md` frontmatter, rejects symlinks/nested skills/name drift, and runs
  in `just test`, `just smoke`, and CI alongside Ruff/ty.
- [ ] S1: Restrict or replace old config env overrides (`CAPSEM_USER_CONFIG`,
  `CAPSEM_CORP_CONFIG`).
- [ ] S1: Run systematic `user.toml` burn audit across code, tests, docs,
  skills, and sprint fixtures. Every `user.toml`, `UserConfig`, and
  `CAPSEM_USER_CONFIG` reference must be deleted, renamed to `settings.toml`,
  moved to profile/corp ownership, or explicitly confined to test/dev-only
  helpers. Production profile routes must not read or write `user.toml`.
- [ ] S1: Replace rule-leaking UI/TUI mutation paths with semantic profile
  facade routes. MCP server/tool, plugin, and skill controls send enum/state
  edits; backend owns translation into profile-owned enforcement, plugin, skill,
  or MCP files. Normal UI/TUI controls must not ship raw rule TOML over routes.
- [ ] S1: Add generic profile mutation service and mutation ledger.
  Route-originated profile changes for MCP, plugins, skills, rules, and future
  profile-owned config must verify current hashes, mutate a single
  profile-owned path, update BLAKE3/size pins, and emit typed mutation-ledger
  rows through the DB writer. Ledger rows must include profile id, category,
  target kind, target key/path, operation, filename, affected file path,
  old/new hash and size, status, and error if any. No ad hoc route file edits
  and no side SQLite writes.
- [ ] S1: Build the profile object abstraction before wiring route mutations:
  `ProfileStore` owns load/lock/verify/save/reload/ledger; `ProfileDocument`
  owns the in-memory `profile.toml` plus referenced sibling files; and
  `ProfileMatrix` exposes the effective UI/TUI/runtime read model. Routes call
  semantic methods such as `set_mcp_tool_permission`, `set_plugin_mode`, and
  `set_skill_enabled`; routes must not parse/write profile files directly.
- [ ] S1: Extend `SecurityRule` with optional typed ownership annotations for
  backend-managed semantic rules. Enforce uniqueness for MCP server/tool,
  plugin, and skill targets so routes update the one owned rule instead of
  searching CEL or inventing new rule names.
- [ ] S1: Add the MCP permission litmus test: changing the `capsem` server's
  `fetch_http` tool to `ask` through the profile MCP tool edit route writes or
  updates the profile enforcement rule, returns `effective_action = "ask"` from
  the tool list, and does not mutate `mcp.json`, `settings.toml`, or any
  `user.toml` path.
- [ ] S1: Add adversarial tests for mutation discipline: stale hash rejects,
  manual file drift rejects, duplicate managed-rule annotations reject,
  unannotated user/corp CEL rules with the same server/tool do not confuse the
  route-owned lookup, and failed mutations are ledgered without partial profile
  file updates.
- [ ] S1: Update code/tests/docs/skills; remove old-path fallbacks.
- [x] S2: Add guest root seed and move CLI config files into real files.
- [x] S2: Add `mcp.json`, `apt-packages.txt`,
  `python-requirements.txt`, `npm-packages.txt`, `install.sh`, and `tips.txt`
  under `config/profiles/code/`.
- [x] S2: Builder copies guest root seed into rootfs seed path.
- [x] S2: `capsem-init` projects seed into runtime `/`.
- [x] S2: In-VM diagnostics assert the projected profile-owned Gemini,
  Antigravity, Claude, Codex, and MCP config files exist, use the profile MCP
  bridge, and contain no obvious credential-shaped secrets.
- [ ] S3: Tool install refresh/version discipline.
  Partial: profile-owned apt/Python/npm package files and `install.sh`
  materialize into the generated guest workspace and rootfs Docker context.
  Open: installed version/hash ledger and real AGY/Codex/Claude/Gemini VM
  proof.
- [ ] S3: Build ledger exposes the packages actually running in the VM:
  declared package input hashes, installed package names, installed versions,
  and local package/artifact hashes where available for apt, Python/uv, npm,
  and manual installers.
- [ ] S3: Use `cdxgen/cdxgen` as the preferred OBOM generator (`obom` /
  `cdxgen -t os`) for the produced Linux rootfs/VM image, and record OBOM path,
  BLAKE3 hash, generator, and generator version in the profile/build evidence.
- [x] S3: Add the profile OBOM contract and runtime API: profile TOML accepts
  `obom.arch.<arch>` descriptors with BLAKE3 hash, size, generator metadata, and
  service/gateway expose `/profiles/{id}/obom` plus `/profiles/{id}/info`.
- [x] S3: Teach `capsem-admin profile materialize` to attach a pinned
  `obom.cdx.json` when the asset manifest lists one; local OBOM documents are
  served only after size and BLAKE3 verification.
- [ ] S4: Documentation and skill cleanup.
- [ ] S4: Update public docs and internal skills after ontology paths land;
  stale `guest/config` guidance is a release hold.
- [ ] S5: Verification gate.
- [ ] S5: Full build gate: rebuild profile assets through the admin/just rail,
  including EROFS/LZ4HC rootfs.
- [ ] S5: Package/install gate: build the real package and install through the
  package path with manifest override support, then verify service/UI readiness.
- [ ] S5: Linux handoff gate: Linux CI/team must run KVM tests for restored
  guest-memory range/overflow hardening because macOS cannot compile/execute
  `hypervisor::kvm` without the Linux toolchain/runtime.
- [ ] S5: Magic inventory gate.
- [ ] Changelog.
  Partial: profile-owned image payload pinning is recorded under Unreleased.
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
- User correction: `user.toml` must burn. User preferences are
  `settings.toml`; profile behavior is profile-owned; corp constraints are
  corp-owned. The current profile enforcement handlers still load/write the old
  settings/user shape internally and must be corrected in S1.
- User correction: UI/TUI must mutate MCP server/tool permissions through
  semantic profile routes. The backend translates simple enum/state edits into
  profile-owned rules/config; do not expose the raw rule system to common UI/TUI
  controls and do not add compound clever routes.
- User correction: semantic route mutations need a mutation ledger. Because
  profile files are hash-pinned, route edits must update the profile ledger and
  emit a DB-writer mutation record with the mutated path and old/new hashes; no
  hand editing, no side writes, no silent hash drift.
- User correction: backend-generated permission rules need typed ownership
  annotations so route code can enforce one rule per semantic target such as MCP
  server/tool. Do not infer route-owned rules by CEL text or naming tricks.
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
- Linux KVM guest-memory safety history check:
  - `0422a6ec fix: validate kvm guest memory ranges` added
    `GuestMemoryRef::gpa_range_to_host` and moved virtio-blk zero-copy/raw
    pointer users from first-byte checks to full-range checks.
  - `45800223 fix: guard kvm memory offset overflow` changed guest memory
    `read_at`/`write_at` arithmetic to checked additions.
  - Both concepts are now ported into the current branch. Local macOS native
    tests cannot execute the KVM module because it is `target_os = "linux"`;
    Linux CI/team validation remains required.
- AGY/Antigravity correction: do not model this as a 48G/64G VM allocation
  change. It is a guest-kernel/runtime option issue to diagnose from a real
  rebuilt-profile boot with `capsem exec` once profile/root/package inputs are
  rebuilt.
- Profile payload slice: `config/profiles/code/profile.toml` now pins MCP,
  package lists, manual installer script, tips, and `root.manifest.json` by
  BLAKE3/size. `root.manifest.json` pins every packaged guest-root file.
  `capsem-admin profile check` verifies both layers and rejects path escape,
  bad hash scheme, zero-size, and mutated payloads.
- Generated image workspace slice: `capsem-admin image build` now validates the
  source profile and materializes `target/image-workspace/<profile_id>/guest`
  from the profile before invoking `capsem-builder`. This is the transition
  rail; the backend still has a `guest_dir` argument and must be burned down to
  an explicit image spec in S1.
- Real arm64 rootfs/initrd build slice:
  `cargo run -p capsem-admin -- image build --profile
  config/profiles/code/profile.toml --config-root config --guest-dir guest
  --output assets --arch arm64 --template rootfs` succeeded through the
  profile-materialized workspace and produced EROFS/LZ4HC level 12
  `assets/arm64/rootfs.erofs` with BLAKE3
  `015b5d930eef2eacfb6b484adaf8abd83cd4fb2c0a4700c24fe696c9db595ba1`, size
  `862875648`. `just _pack-initrd` then repacked diagnostics into
  `assets/arm64/initrd.img` with BLAKE3
  `7928dd872e09c33ca001f779d987cb7b71d3df8f3f9ed74ca68aeb5c38d1fb9f`, size
  `2849956`. The profile asset pins were reconciled to the generated manifest
  for arm64 `initrd.img` and `rootfs.erofs`.
- Runtime projection gotcha: profile root files are baked into EROFS, but
  `capsem-init` overlays diagnostics from initrd at boot for fast iteration.
  Therefore profile-root changes require a rootfs rebuild, and diagnostic test
  changes require `just _pack-initrd`; otherwise doctor may execute stale tests
  against fresh profile files.
- Installer proof: the first real Docker build failed because downloaded
  installer scripts were executed with `/bin/sh`; the profile `install.sh` now
  invokes them with Bash. The retry installed Claude Code `2.1.170` and
  Antigravity CLI `1.0.7` during the rootfs build.
- VM proof: isolated dev service under `target/capsem-dev-home` booted the
  rebuilt arm64 profile assets. `capsem status` reported Capsem
  `1.3.1781050981`, assets manifest `2026.0609.18`, `1/1` profile ready, and
  arm64 vmlinuz/initrd/rootfs all `ok`. `capsem doctor --fast` passed with
  `286 passed, 23 skipped, 1 deselected in 27.04s`; log:
  `target/capsem-dev-home/run/doctor-latest.log`.
- Size/performance note for follow-up, not a blocker for this proof: because
  the profile-root layer currently sits before Python/package-heavy Docker
  layers, small profile-root edits can invalidate expensive image layers; NVM
  and Python packages also include test/data trees that may be pruneable later.
- Verification for this slice:
  - `cargo test -p capsem-core --lib -- --nocapture` passed with 1506 tests,
    1 ignored.
  - `cargo fmt --check` passed.
  - `git diff --check` passed.
  - `cargo test -p capsem-core hypervisor::kvm::memory -- --nocapture`
    compiled the crate but ran zero KVM tests on macOS, then hit one transient
    codesign failure for the unrelated `mcp_export` test binary.
  - `cargo test -p capsem-core hypervisor::kvm::virtio_blk -- --nocapture`
    completed with zero KVM tests on macOS because the module is Linux-only.
  - Linux cross-check attempts are recorded in the coverage ledger below.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-core profile_contract -- --nocapture`
  proves profile file refs parse, serde/validate, and reject absolute paths,
  traversal, bad hash schemes, and zero-size pins. Restored KVM memory tests
  exist in `memory.rs`/`virtio_blk.rs` but are Linux-only.
- Tooling: `uv run ruff check .` and `uv run ty check src/capsem` are the
  current Python quality gates.
- Skill contract: `uv run capsem-builder validate-skills config/skills` and
  `uv run python -m pytest tests/test_skills.py -q` pass. The validator is
  Pydantic-backed and wired into local/CI gates.
- Profile-directory contract: `cargo test -p capsem-core profile_contract -- --nocapture`,
  `cargo test -p capsem-admin -- --nocapture`, `cargo test -p capsem-service
  profile_catalog -- --nocapture`, and the focused package/install pytest set
  pass after moving source and generated profiles to
  `profiles/<id>/profile.toml`.
- Functional: `cargo run -p capsem-admin -- profile check
  config/profiles/code/profile.toml --config-root config --arch arm64 --json`
  reports every profile payload and packaged-root file with matching
  BLAKE3/size. `cargo test -p capsem-admin
  image_workspace_materializes_self_contained_profile_config -- --nocapture`
  proves image workspace materialization.
- Auditability: backend build-ledger tests prove JSONL emission for rendered
  Dockerfile/build-context hashes, rootfs tar, EROFS, kernel assets, and tool
  versions. Pending: profile/payload hash records once profile hash schema
  lands.
- Adversarial: `cargo test -p capsem-admin profile_check -- --nocapture`
  proves mutated profile payload files are rejected and profile root manifests
  are verified. Remaining: checked-in credential sweep for
  `config/profiles/<profile_id>/root/`.
- E2E/VM: isolated rebuilt-profile boot passed `capsem doctor --fast` with
  `286 passed, 23 skipped, 1 deselected in 27.04s`. The doctor suite now proves
  profile-owned Gemini, Antigravity, Claude, Codex, and MCP config files exist
  in runtime `/root`, use the canonical `/run/capsem-mcp-server` bridge where
  applicable, and contain no obvious credential-shaped secrets.
- Asset build: arm64 rootfs rebuild through `capsem-admin image build` passed,
  and `cargo run -p capsem-admin -- image verify --profile
  config/profiles/code/profile.toml --config-root config --output assets
  --manifest assets/manifest.json --arch arm64 --json` passed for vmlinuz,
  initrd, and rootfs pins.
- Linux/KVM: local macOS cannot execute KVM tests. Attempted
  `cargo check -p capsem-core --target x86_64-unknown-linux-gnu`, blocked
  because the target is not installed; attempted
  `cargo check -p capsem-core --target x86_64-unknown-linux-musl`, blocked by
  missing `x86_64-linux-musl-gcc` for C dependencies (`libsqlite3-sys`, `ring`,
  `aws-lc-sys`). Linux CI/team must run this gate.
- Telemetry: not directly touched unless doctor/status output changes.
- Performance: tool refresh may affect image build time; runtime should not add
  hot-path latency. `uv run python -m pytest tests/test_docker.py -q` passes
  with 148 backend builder/context tests and no Docker execution.
- Missing/deferred: none accepted yet.
