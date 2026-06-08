# Snapshot Restore Tracker

## S0: Inventory And Classification

- [x] Capture `git diff --name-status 82e7a58c^1 82e7a58c` into this
  sub-sprint or a generated evidence file. Evidence:
  `S0-loss-inventory.md`.
- [x] Mark every deleted cluster as exact restore, conceptual port,
  intentional burn, or Linux handoff. Initial capability-level classification
  is in `S0-loss-inventory.md`; commit-by-commit ledger remains open below.
- [x] Confirm restore work will not change the current security event object,
  plugin contract, rule format, detection format, or plugin/rule/detection
  corp/profile file locations. If blocked, stop and ask; no schema migration
  escape hatch.
- [x] Confirm corp rules may use negative priority. If a corp rule omits
  `priority`, it resolves to the corp source default (`-10`).
  `priority = "default"` remains profile/built-in fallback only.
- [x] Confirm corp source implies corporate lock/ownership. Do not require or
  accept `corp_locked = true` inside corp-owned rule files.
- [x] Confirm old policy-v2/domain/MCP decision rails stay burned.
- [x] Confirm old `capsem setup` and provider onboarding wizard stay burned.
- [x] Confirm `[credentials] broker_enabled` stays burned; credential brokering
  is owned only by `[plugins.credential_broker]`.
- [x] Confirm static `[ai.*]` provider metadata stays burned unless it is
  replaced by real provider status computed from rules, VM plugin runtime
  status, observed tool config hashes, routing config, and runtime security
  events.
- [x] Confirm old `config/defaults.toml` `settings.ai.*` defaults and
  host-credential injection blocks are burned or reshaped into profile-owned
  rules plus plugin-owned runtime status. They must not remain UI settings.
- [x] Burn generated/runtime settings-owned AI provider registry. Decision:
  intentional_burn. `config/defaults.toml`, generated defaults JSON, generated
  mock settings, frontend settings-store/model tests, integration config
  fixtures, and the settings architecture page no longer expose
  `settings.ai.*` provider toggles/API keys/domains. Loader and inline corp
  validation reject retired flat AI setting IDs. Coverage:
  `just _generate-settings`, `cargo test -p capsem-core --lib policy_config --
  --nocapture`, `uv run pytest tests/test_config.py -q`, `pnpm -C frontend
  check`, and `pnpm -C frontend test
  src/lib/models/__tests__/settings-model.test.ts
  src/lib/__tests__/settings-store.test.ts`.
- [x] Burn stale settings-based API-key injection tests. Decision:
  intentional_burn. Removed `tests/test_api_key_injection.sh` and the old
  Python E2E that expected broker references in guest env; broker/plugin
  behavior remains covered in credential broker, fs monitor, security engine,
  and MITM telemetry hook tests.
- [x] Burn retired service-global asset status helper. Decision:
  intentional_burn. Removed the dead `asset_status_value` helper and converted
  reconcile-progress coverage to `profile_asset_status_value` over the
  profile-owned hash-prefixed asset contract. Coverage:
  `cargo test -p capsem-service asset_status_reports_reconcile_progress_fields
  -- --nocapture`, `cargo test -p capsem-service --no-run`, and `uv run pytest
  tests/capsem-service/test_svc_install.py tests/capsem-service/test_svc_mcp_api.py -q`.
- [x] Follow-up: sweep remaining Python integration/gateway VM creation
  fixtures so every `/vms/create` payload carries explicit `profile_id =
  "code"` or intentionally asserts the missing-profile rejection. Also made
  one-shot `/run` tests profile-explicit after the real service rejected the
  old payload shape, and tightened the gateway mock so `/vms/create` and
  `/run` reject missing profile ids. Coverage: read-only payload sweep over
  `/vms/create` and `/run`, `git diff --check`, `uv run pytest
  tests/capsem-gateway/test_gw_proxy.py tests/capsem-gateway/test_gw_proxy_advanced.py
  tests/capsem-gateway/test_gw_concurrent.py -q`, `uv run pytest
  tests/capsem-service/test_svc_provision.py tests/capsem-service/test_svc_exec_ready.py
  tests/capsem-service/test_svc_fork.py tests/capsem-service/test_svc_startup.py -q`,
  `uv run pytest tests/capsem-service/test_svc_persistence.py
  tests/capsem-service/test_svc_resume_paths.py -q`, `uv run pytest
  tests/capsem-service/test_svc_suspend_corruption.py
  tests/capsem-service/test_svc_loop_device_after_resume.py -q`, `uv run pytest
  tests/capsem-gateway/test_mitm_policy.py -q`, and `uv run pytest
  tests/capsem-e2e/test_framed_mcp_mitm.py --collect-only -q`.
- [x] Commit S0. Evidence and S0 cleanup slices are committed through
  `25b8b326 docs: align 1.3 contracts`; S0 tracker closure is committed in
  `70638109 chore: close snapshot restore s0`; worktree was clean before
  entering the S1 commit ledger.

## Commit Inspection Ledger

Each checkbox means we inspected the commit and recorded one of:
`exact_restore`, `conceptual_port`, `intentional_burn`, or `linux_handoff`.
Write the decision inline after the checkbox before marking it complete, for
example:

```text
- [x] `048d7cf5 ...` decision: conceptual_port. Notes: restore
  profile-selected asset requirements, but wire them into current profile
  routes and asset manager.
```

Do not check a commit just because a later commit appears to supersede it. If it
introduced a test, contract, command, or benchmark, inspect it and either port
the guarantee or explicitly burn it.

### S1 Profile/Admin/Asset Pipeline Commits

- [ ] `9ca1bbed release: v1.2.1779658398`
- [ ] `1bdd27cb bench: record macos arm64 benchmark results`
- [ ] `89b04f87 perf: tune rootfs squashfs block size`
- [ ] `6823cf1f feat: package capsem tui binary`
- [ ] `03fcce34 fix: skip asset alias directories in install profiles`
- [ ] `b8ca8589 fix: ignore manifest aliases in install profiles`
- [ ] `6daf264a fix: point package profiles at release assets`
- [ ] `a841716f fix: sign packaged admin python extensions`
- [ ] `718981b1 docs: record admin release gate proof`
- [ ] `24c846e8 refactor: rename admin policy packs to enforcement`
- [ ] `923d603f test: add session process policy corpus`
- [ ] `63eccc3f feat: support admin model tool policy paths`
- [ ] `9944c7ba feat: expand admin policy context parity`
- [ ] `391eaece fix: compile-check policy backtests before replay`
- [ ] `b07101ed test: tighten admin policy path compile`
- [ ] `2f9b0fd0 test: expand s08c policy corpus diversity`
- [ ] `80a416be feat: add admin policy compile`
- [ ] `2db1259a test: pin s08c detection ir parity`
- [ ] `099152a4 feat: add admin policy backtest corpus`
- [ ] `7b14ccb4 feat: add admin detection backtest corpus`
- [ ] `2bedce99 feat: seed policy context rule corpus`
- [ ] `b0eecdd7 feat: add admin doctor closeout`
- [ ] `0e1e6b1b feat: add detection ir parity`
- [ ] `66141eee feat: compile detection packs`
- [ ] `d773481f feat: validate security packs`
- [ ] `7277c17b feat: generate guest image sboms`
- [ ] `3a37d704 feat: verify doctor bundle probes`
- [ ] `2d02b6e0 fix: require image inventory proof`
- [ ] `33c83bd0 feat: verify per-arch image inventories`
- [ ] `a1dab24f feat: extract image inventory from rootfs`
- [ ] `0ffb816a feat: verify image package inventory`
- [ ] `c9fd7b4b feat: require profiles for asset builds`
- [ ] `fd86e8ed feat: derive built-in profiles from guest config`
- [ ] `5b4e4274 feat: generate profile ui base profiles`
- [ ] `a02537ad feat: add profile-derived image build command`
- [ ] `31425d04 feat: materialize profile image workspaces`
- [ ] `879c9d59 test: prove packages include capsem-admin`
- [ ] `22016426 feat: add capsem-admin manifest crypto`
- [ ] `6559bf3b feat: add capsem-admin manifest generate`
- [ ] `3e5bb3cb feat: add capsem-admin manifest download check`
- [ ] `e2946acd feat: add capsem-admin manifest fast check`
- [ ] `2cc49f7a feat: add capsem-admin image verify`
- [ ] `2fb45076 feat: add capsem-admin image plan`
- [ ] `0e9442e4 test: pin admin init json toml parity`
- [ ] `53065265 test: pin profile toml json round trip`
- [ ] `c9e227c1 test: pin service settings toml json round trip`
- [ ] `839c1114 feat: add capsem-admin settings init`
- [ ] `d2834490 feat: add capsem-admin profile init`
- [ ] `be6909a0 feat: add profile section editability gates`
- [ ] `634b9730 feat: add capsem-admin profile validation`
- [ ] `810b417a test: pin service settings default parity`
- [ ] `d0c1c988 feat: wire capsem-admin settings commands`
- [ ] `d39756f3 feat: add service settings admin contract`
- [ ] `be0741e1 feat: verify admin profile payload installs`
- [ ] `25eb08d9 feat: align admin profile lifecycle gates`
- [ ] `f3fdbf0a chore: make profile manifest canonical`
- [ ] `b04cb88c feat: add pydantic profile contracts`
- [ ] `a8f712d5 feat: add profile v2 schema artifact`
- [ ] `4cdba35f refactor install asset prep into scripts`
- [ ] `d4d2bb3a fix: harden release package verification`
- [ ] `5d7e58ce fix: harden installer downloads and release package checks`
- [ ] `22096b7f fix: harden release install deb repack`

### S2 Runtime Profile Assets/Pins Commits

- [x] Current-architecture slice: VM creation now requires a real profile id
  and persists it through runtime state, persistent registry rows, fork, save,
  resume, list, and info. Decision: conceptual_port of the lost
  profile-selected create/lineage guarantees into the current profile catalog.
  Tests: `cargo test -p capsem-service profile_id -- --nocapture`,
  `cargo test -p capsem-service profile -- --nocapture`, targeted
  `provision_rejects_unknown_profile_before_boot`,
  `provision_rejects_source_with_different_profile`,
  `handle_fork_creates_persistent_sandbox`,
  `handle_fork_from_persistent_registry`,
  `handle_persist_preserves_profile_identity`, and
  `sandbox_info_rejects_missing_profile_id`.
- [x] Current-architecture slice: VM boot preflight and process spawn now
  resolve kernel/initrd/rootfs from the selected profile's current-arch asset
  descriptors. Decision: conceptual_port of profile-selected boot assets into
  current `ProfileConfigFile`/`ProfileCatalog`; old service-global asset
  guessing no longer drives create/run/resume boot. The resolver accepts
  hash-prefixed downloaded assets and logical-name dev assets only when they
  derive from the profile descriptor. Tests: `cargo test -p capsem-service
  resolve_profile_asset_paths_uses_profile_hash_prefixed_assets -- --nocapture`,
  `cargo test -p capsem-service vm_asset_block_reason -- --nocapture`,
  `cargo test -p capsem-service --no-run`, and `cargo test -p capsem-service
  profile -- --nocapture`.
- [x] Current-architecture slice: `/profiles/{profile_id}/assets/ensure` now
  downloads and verifies the selected profile's current-arch asset descriptors
  directly, writes hash-prefixed targets, updates reconcile status, and skips
  already-verified files. Decision: conceptual_port of profile-owned asset
  reconcile/download into current profile contract; old manifest-global ensure
  no longer drives the profile ensure route. Tests: `cargo test -p
  capsem-service ensure_profile_assets_downloads_profile_descriptors --
  --nocapture`, `cargo test -p capsem-service --no-run`, and `cargo test -p
  capsem-service profile -- --nocapture`.
- [x] Current-architecture slice: persistent VM rows and live runtime state now
  carry the selected profile revision, typed profile payload BLAKE3 hash, plus
  kernel/initrd/rootfs boot asset pins. Create/save/fork/resume preserve those
  pins, while resume rejects profile revision or payload hash drift and
  fork/save reject current profile asset-pin drift before booting or cloning
  stale state. Decision: conceptual_port of persistent VM profile
  revision/payload/base-asset pinning into the current profile catalog and
  registry contract; byte-level asset verification remains owned by profile
  asset ensure/download. Tests: `cargo test -p capsem-service
  resume_rejects_profile_revision_drift -- --nocapture`, `cargo test -p
  capsem-service resume_rejects_profile_payload_hash_drift -- --nocapture`,
  `cargo test -p capsem-service
  persistent_vm_entry_rejects_missing_profile_contract_fields -- --nocapture`,
  `cargo test -p capsem-service handle_fork_rejects_asset_pin_drift --
  --nocapture`, `cargo test -p capsem-service
  handle_persist_preserves_profile_identity -- --nocapture`, `cargo test -p
  capsem-service handle_fork -- --nocapture`, `cargo test -p capsem-service
  profile -- --nocapture`, and `cargo test -p capsem-service --no-run`.
- [x] Current-architecture cleanup slice: root `config/` now contains only
  real configuration/generator outputs. MITM CA key material lives under
  `security/keys/`; retired settings presets and their Rust/Python/
  frontend schema hooks are burned. Decision: intentional_burn for the preset
  subsystem, conceptual cleanup for key placement so profile/corp/config
  ownership is not confused by CA artifacts. Tests:
  `cargo test -p capsem-core --lib policy_config -- --nocapture`, `cargo test
  -p capsem-core --lib manifest -- --nocapture`, `cargo test -p capsem-core
  --lib cert_authority -- --nocapture`, `uv run pytest
  tests/test_settings_spec.py tests/test_config.py
  tests/test_docker.py::TestGenerateChecksums
  tests/test_docker.py::TestPrepareBuildContextArtifacts tests/test_doctor.py
  tests/capsem-rootfs-artifacts/test_rootfs_artifacts.py -q`, `pnpm -C
  frontend test src/lib/models/__tests__/settings-model.test.ts
  src/lib/__tests__/settings-store.test.ts`, `git diff --check`, and a
  targeted `rg` sweep for the old root-config signing/CA/preset paths and
  preset action symbols.
- [x] Current-architecture cleanup slice: profile asset descriptors are now
  only role/name/url/hash/size. Removed fake per-asset signature/content-type
  metadata and removed filesystem/compression/compression-level build knobs
  from profile payloads and profile asset status responses. Runtime reads
  manifest metadata only for BLAKE3 hash lookup; release evidence is
  SBOM/provenance plus profile/corp URL selection and BLAKE3 byte
  verification. Tests: `cargo test -p capsem-core
  --lib profile_contract -- --nocapture`, `cargo test -p capsem-core --lib
  manifest -- --nocapture`, `cargo test -p capsem-core --lib policy_config --
  --nocapture`, `cargo test -p capsem-service
  profile_assets_info_reflects_manifest_and_edit_is_gated -- --nocapture`,
  `cargo test -p capsem-service
  profile_asset_status_uses_profile_current_arch_contract -- --nocapture`,
  `cargo test -p capsem-service profile -- --nocapture`, `git diff --check`,
  and targeted `rg` sweeps for manifest signing and removed profile asset
  fields.
- [ ] `b2fb7e33 feat: export session policy contexts`
- [ ] `7a5afc9c test: prove process enforcement logs in real vm`
- [ ] `f2a6247f docs: close s07 debt ledger`
- [ ] `f5aea0fc test: gate release image boot proof`
- [ ] `dcba8776 feat: harden profile trust and policy runtime`
- [ ] `e3be977e feat: prove s08 profile-selected gateway create`
- [ ] `694aa75b feat: select profiles during vm create`
- [ ] `2a1d079d test: prove vm fork lineage`
- [x] `204ce825 feat: schedule profile catalog reconciliation` decision:
  conceptual_port. The old scheduled remote manifest reconciler depended on
  deleted profile-manifest/settings-profile infrastructure, so this slice adds
  explicit current-contract catalog status/reload routes instead:
  `GET /profiles/status` and `POST /profiles/reload` validate the active
  `ProfileCatalog`, expose source/profile counts, and summarize per-profile
  readiness through the same profile asset contract used by boot. Tests:
  `cargo test -p capsem-service
  handle_profiles_status_reports_builtin_catalog_readiness -- --nocapture`,
  `cargo test -p capsem-service
  profile_catalog_status_reports_directory_catalog_readiness -- --nocapture`,
  `cargo test -p capsem-service
  profile_catalog_reload_rejects_invalid_directory_catalog -- --nocapture`,
  `cargo test -p capsem-service profile -- --nocapture`, and `cargo test -p
  capsem-service --no-run`.
- [ ] `438c9642 feat: fetch profile catalogs from URL`
- [ ] `3204f27a test: prove profile asset boot flow`
- [x] `95155405 feat: expose profile asset provenance` decision:
  conceptual_port. Current `/profiles/{profile_id}/assets/status` now exposes
  profile revision, typed profile payload hash, descriptor provenance, and
  present/missing state through the same hash-prefixed resolver used by boot,
  rather than restoring the old asset supervisor shape. Tests: `cargo test -p
  capsem-service profile_asset_status_uses_profile_current_arch_contract --
  --nocapture`, `cargo test -p capsem-service
  ensure_profile_assets_downloads_profile_descriptors -- --nocapture`,
  `cargo test -p capsem-service profile -- --nocapture`, and `cargo test -p
  capsem-service --no-run`.
- [ ] `0a87e26a test: harden profile asset reconcile races`
- [ ] `deb1b083 refactor: remove legacy asset manifest runtime`
- [ ] `d069710f feat: trigger profile asset reconcile from update`
- [x] `2d7e1470 feat: derive profile asset retention roots` decision:
  conceptual_port. The current tree no longer has the old `saved_vm_assets.rs`
  shape, so cleanup now accepts an explicit preserve set and service startup
  derives that set from the active profile catalog plus persistent VM boot
  asset pins before deleting stale hash-prefixed files. Tests: `cargo test -p
  capsem-core cleanup_preserves_explicit_retention_filenames -- --nocapture`,
  `cargo test -p capsem-service
  asset_cleanup_preserves_profile_catalog_and_persistent_vm_pins --
  --nocapture`, `cargo test -p capsem-core cleanup -- --nocapture`, `cargo
  test -p capsem-service profile -- --nocapture`, and `cargo test -p
  capsem-service --no-run`.
- [ ] `911d6a67 feat: fetch signed profile payloads`
- [ ] `dd42a2d4 feat: verify profile payload signatures`
- [ ] `237d2bbc feat: materialize verified profile payloads`
- [ ] `152c7780 feat: verify installable profile payloads`
- [ ] `d50d8a13 feat: add profile catalog lifecycle gates`
- [ ] `048d7cf5 feat: drive runtime assets from profiles`
- [ ] `d759668c feat: validate profile payload schema in rust`
- [ ] `996de225 feat: add profile manifest catalog types`
- [ ] `f3578c3d release-debug-loop: finalize saved VM asset tracking and status surfaces`

### S3 TUI/Shell And Lower-Priority Debug Commits

- [ ] `0a425541 chore: merge main into tui control`
- [ ] `a476d7a7 chore: merge main into tui control branch`
- [ ] `9ca1bbed release: v1.2.1779658398`
- [ ] `32102d6d fix: purge broken persistent tui sessions`
- [ ] `2b6a2edc fix: offer tui recovery create and purge`
- [ ] `0cf0a9a0 fix: keep tui create focus pending`
- [ ] `6902dc4b fix: show full-screen tui suspend progress`
- [ ] `b50c811d fix: reconnect tui terminal after resume`
- [ ] `9b168fd5 fix: focus tui create and hide corrupt tabs`
- [ ] `860cc8ea feat: make capsem shell launch tui`
- [ ] `f3068301 fix: prompt tui service start when offline`
- [ ] `53862ec2 fix: block tui create without profiles`
- [ ] `92143119 fix: open tui new session on empty state`
- [ ] `c2fb4b77 fix: move tui help hint to session stats`
- [ ] `e3d0312f fix: polish tui controls and overlays`
- [ ] `fb98b2d1 fix: add tui fork flow`
- [ ] `f5a73773 fix: make tui create profile aware`
- [ ] `d47a889a fix: pin tui suspend hint left`
- [ ] `f60bb671 fix: surface tui suspend shortcut`
- [ ] `1299bd5c fix: render stopped tui sessions`
- [ ] `6138c0b9 fix: gate endpoint latency hot paths`
- [ ] `a21e269c fix: stabilize tui latency display`
- [ ] `161e40f4 fix: simplify tui tab colors and modal input`
- [ ] `43716abb fix: harden tui modal and resize behavior`
- [ ] `91a9cf93 fix: make tui shell controls alt-only`
- [ ] `f54d94a0 fix: stabilize tui session navigation`
- [ ] `ec0c7152 fix: use vt parser for tui terminal`
- [ ] `c93351ee fix: finish tui live terminal proof`
- [ ] `6823cf1f feat: package capsem tui binary`
- [ ] `ec473982 feat: add confirmed capsem tui service actions`
- [ ] `92a9992f feat: add capsem mcp terminal snapshot`
- [ ] `921b941f feat: add capsem tui gateway terminal shell`
- [ ] `2e79056b style: simplify capsem tui chrome`
- [ ] `c6a70081 feat: add standalone capsem tui shell`
- [ ] `1845ec83 fix: stop install harness service before error tests`
- [ ] `33684fcd fix: compile debug report disk stats on macos`
- [ ] `2322fbf2 feat: surface security health in status`
- [ ] `27e985d8 feat: expose runtime security debug health`
- [ ] `ddaf358c test: extend s08 gateway diagnostics coverage`
- [ ] `be5f902b feat(settings-profiles): add debug provenance`
- [ ] `77ec3abf feat: add structured debug report`
- [ ] `fe7a4071 fix: harden local install diagnostics`
- [ ] `9713a49e fix(setup): split install vs. onboarding flags so reinstall stops re-showing wizard`
- [ ] `0dd1d8ed test(install): self-heal layout fixture, gate intrusive auto-launch tests`
- [ ] `5c897436 fix: switch pytest to importlib mode + package-relative conftest imports`
- [ ] `ae888779 feat: wire real .pkg/.deb install paths, harden installer pipeline`
- [ ] `6c1a639e feat: capsem setup interactive wizard`

### S4 Linux/KVM/EROFS/LZ4HC/Benchmark Commits

- [ ] `0a425541 chore: merge main into tui control`
- [ ] `9ca1bbed release: v1.2.1779658398`
- [ ] `4d133bb7 bench: rerun mac benchmark after linux merge`
- [ ] `b4ba5ce6 bench: record linux wrap-up benchmark artifacts`
- [ ] `b6f9b6e2 bench: preserve artifacts before benchmark reruns`
- [ ] `8e8c4a77 bench: archive superseded benchmark artifacts`
- [ ] `05df4127 docs: add hypervisor improvement sprint`
- [ ] `56b61a22 bench: record default off io_uring results`
- [ ] `803bfbac perf: make kvm io_uring block opt in`
- [ ] `7233acf9 bench: record gated kvm io_uring results`
- [ ] `c2422adf perf: gate kvm io_uring block to writable disks`
- [ ] `a0ef66bb bench: record kvm io_uring block results`
- [ ] `7037bac3 perf: add kvm virtio block io_uring backend`
- [ ] `0bbd5397 bench: record virtio block telemetry results`
- [ ] `4ca0fb0a feat: add kvm virtio block telemetry`
- [ ] `a0f8df6b bench: record kvm event index results`
- [ ] `3b2c7390 perf: add kvm virtio block event index`
- [ ] `9d4c1f2a bench: record combined kvm block stack results`
- [ ] `ba8f260e perf: combine kvm ioeventfd block batching`
- [ ] `20bb3483 Revert "perf: route kvm block notify through ioeventfd"`
- [ ] `7e7c470c perf: route kvm block notify through ioeventfd`
- [ ] `14dc4562 Revert "perf: batch kvm block used ring updates"`
- [ ] `589494f5 perf: batch kvm block used ring updates`
- [ ] `2d56217c Revert "perf: move kvm block io off vcpu notify"`
- [ ] `8a391cb1 perf: move kvm block io off vcpu notify`
- [ ] `c4b07da8 bench: record vectored kvm block io results`
- [ ] `0dbd5099 perf: use vectored kvm block io`
- [ ] `c093f4b4 bench: include storage diagnostics in canonical run`
- [ ] `f4308f01 perf: trim kvm rootfs overlays before fork`
- [ ] `4c75cbfe bench: enforce benchmark artifact contract`
- [ ] `d5f67d78 bench: compare linux and mac artifacts`
- [ ] `968ae891 bench: archive criterion artifacts`
- [ ] `ab03714d bench: record linux benchmark artifacts`
- [ ] `d56e07ac bench: parse git status paths correctly`
- [ ] `67add8b4 bench: distinguish source dirtiness in artifacts`
- [ ] `8286bd34 bench: use project filesystem for native baseline`
- [ ] `8e4e645d bench: record host native baselines`
- [ ] `5b9ee2c2 bench: standardize benchmark recipe`
- [ ] `3d5a8745 bench: split rootfs workload diagnostics`
- [ ] `a52f7aab perf: negotiate larger virtiofs requests`
- [ ] `b9716188 perf: use positional virtiofs io`
- [ ] `31b96ebd bench: record storage tuning context`
- [ ] `d3c7d6d2 bench: profile storage iops`
- [ ] `9e996102 bench: add storage split diagnostics`
- [ ] `f4ea4037 test: harden linux benchmark artifacts`
- [ ] `d9429e1f fix: stabilize linux kvm test gate`
- [ ] `5a1397f1 fix: resume kvm guests from warm checkpoints`
- [ ] `3bf9f18f fix: expand kvm warm restore state`
- [ ] `bdedb26a fix: preserve kvm vcpu mp state in checkpoints`
- [ ] `e34817ae docs: record linux kvm doctor pass`
- [ ] `e046977e test: cover tmp symlinks in linux kvm doctor`
- [ ] `61b775a2 fix: trust git workspaces in linux kvm guests`
- [ ] `6be2d86a fix: keep uv cache off virtiofs workspace`
- [ ] `eb76d419 fix: use linux readlink opcode for virtiofs`
- [ ] `5cee8c99 fix: preserve virtiofs inode paths on rename`
- [ ] `06cc31e5 feat: checkpoint linux kvm proving ground`
- [ ] `ea1e7e6c test: align release gate with hardened cli`
- [ ] `49bcf13d test: stabilize release gate hot paths`
- [ ] `cffc9fbf chore: checkpoint remaining S5/S6 backend and artifact updates`
- [ ] `c215b6d9 fix: keep pr linux kvm tests compile-only`
- [ ] `41be412a fix: restore linux kvm test compilation`
- [ ] `92a388ef chore(bench): refresh fork/lifecycle/capsem-bench data snapshots`
- [ ] `ffef142b test(bench): add parallel VM benchmark + preserve-always tmp dir flag`
- [ ] `48104328 refactor: move inline test modules to sibling tests.rs files`
- [ ] `e7a80751 feat(tests): archive in-VM capsem-bench baseline on every just test`
- [ ] `2d94b0a9 chore(bench): record 1.0.1776445634 lifecycle and fork bench data`
- [ ] `ae888779 feat: wire real .pkg/.deb install paths, harden installer pipeline`
- [ ] `2e4a7a50 docs: update benchmark data for 0.16.1`
- [ ] `662edecc fix: cold boot 6x faster (6.2s -> 1.0s), deduplicate backoff`
- [ ] `9b110812 docs: fork benchmark data, results page, and release process updates`
- [ ] `031aafa6 feat: v0.16.1 -- KVM diagnostics, doctor rewrite, platform-specific boot errors`
- [ ] `dae43aa9 fix: optional PIT for CI KVM, boot test in cross-compile, GNU cross-linker`
- [ ] `6039e821 fix: x86_64 Linux build -- cfg-gate aarch64 boot module, cross-linker config`
- [ ] `717d03e5 feat: x86_64 KVM boot fixes, arch validation, cross-compile Docker image`
- [ ] `f68bc9fc feat: x86_64 release boot test, compile-time KVM guardrails, arch-mismatch detection`
- [ ] `db1a82c5 feat: add x86_64 KVM backend -- bzImage boot, IRQCHIP, 16550 UART, PIO bus`
- [ ] `5811282e feat: capsem-builder integration, multi-arch CI, per-arch asset layout`
- [ ] `3cb8e44a feat: hypervisor abstraction layer with Apple VZ and KVM backends`
- [ ] `525b59bf feat: async VirtioFS worker thread with irqfd interrupts`

### S5 Security Corpus/Rules/Bench Commits

- [ ] `24c846e8 refactor: rename admin policy packs to enforcement`
- [ ] `923d603f test: add session process policy corpus`
- [ ] `63eccc3f feat: support admin model tool policy paths`
- [ ] `9944c7ba feat: expand admin policy context parity`
- [ ] `391eaece fix: compile-check policy backtests before replay`
- [ ] `b07101ed test: tighten admin policy path compile`
- [ ] `2f9b0fd0 test: expand s08c policy corpus diversity`
- [ ] `80a416be feat: add admin policy compile`
- [ ] `2db1259a test: pin s08c detection ir parity`
- [ ] `099152a4 feat: add admin policy backtest corpus`
- [ ] `7b14ccb4 feat: add admin detection backtest corpus`
- [ ] `2bedce99 feat: seed policy context rule corpus`
- [ ] `0e1e6b1b feat: add detection ir parity`
- [ ] `66141eee feat: compile detection packs`
- [ ] `d773481f feat: validate security packs`

## S1: Profile/Admin Command Spine

- [ ] Restore base profile files as profile-owned release inputs.
- [x] Write canonical `config/settings.toml`, `config/profiles/code.toml`, and
  `config/corp.toml`; remove stale `config/user.toml.default`.
- [ ] Restore profile/settings schemas and fixtures updated to the modern 1.3
  profile contract.
- [ ] Restore per-architecture profile asset declarations, top-level
  `refresh_policy`, and `[assets].refresh_policy` in profile syntax. Channel,
  manifest URL, and trust keys are catalog/manifest fields, not profile payload
  fields.
- [ ] Restore release/profile evidence chain: release artifacts carry SBOM and
  provenance, corp/profile config owns asset URLs and refresh policy, and
  profile-selected assets are verified by BLAKE3 hash.
- [ ] Ensure profile syntax carries modern default rules, enforcement rules,
  detection levels, provider control rules, MCP, and plugin config.
- [x] Do not add a credential broker invocation rule. `[plugins.credential_broker]`
  governs broker behavior; the broker owns its HTTP-boundary materialization
  hook internally.
- [x] Enforce the plugin contract: plugins own their own filtering/scope and
  materialization hooks. CEL rules do not invoke plugins.
- [x] Preserve the rule/plugin boundary: if behavior can be expressed as a
  CEL/Sigma rule, it is a rule; plugins are only for mutation, materialization,
  external scanning, credential substitution, protocol rewrites, or other
  audited side effects.
- [ ] Extend the plugin object contract with `id`, `name`, `description`,
  `info`, `version`, `mode`, `detection_level`, typed `stages`,
  plugin-owned `scope`, `status_schema`, `stats_schema`, benchmark spec, and
  declared `supports` capabilities.
- [ ] Define plugin stages as a typed enum, not strings in call sites:
  `pre_decision`, `post_decision`, and `runtime_status`. Tests must prove the
  UI/API can tell whether each plugin runs before enforcement, after
  enforcement, or only reports runtime state.
  - Engine side now has typed `SecurityPluginStage::{PreDecision,PostDecision}`;
    descriptor/API exposure and `runtime_status` remain open.
- [ ] Replace the current service `plugin_catalog()` tuple shape with a typed
  plugin descriptor/registry. The descriptor owns `name`, `description`,
  `info`, `version`, stages, status schema, stats schema, benchmark spec,
  capability list, and default config so UI/API surfaces reflect plugin truth
  rather than invented labels.
- [ ] Add plugin descriptor contract tests proving every registered plugin has
  a stable id, semver version, name, description, info, at least one stage,
  status schema, stats schema, benchmark spec, and supported capability list.
- [ ] Ensure profile/corp plugin config tracks policy/config only. Plugin
  registry/runtime owns name, description, info, status schemas, and capability
  metadata for UI reflection.
- [ ] Add plugin benchmark discovery and execution tests. Benchmarks must
  report plugin id, version, stage, fixture id, event count, latency, mutation
  count, and error count. Keep them fast enough for local release smoke.
- [ ] Add required plugin runtime performance counters: invocation count,
  match/skip count, mutation count, allow/ask/block/rewrite count, error count,
  total latency, p50/p95/p99 latency, max latency, and per-stage latency.
- [ ] Add plugin latency attribution tests using dummy plugins: a fast no-op,
  a mutating plugin, and an intentionally delayed plugin. Tests must prove
  counters identify which plugin/stage added latency without reading the DB.
- [ ] Add profile plugin lifecycle routes: list, add, info, edit, delete, and
  reload.
- [ ] Add VM plugin runtime routes: list, status, stats, and reload where the
  plugin supports reload.
- [ ] Enforce HTTP gateway explicit-route allowlist. Every reachable service
  route must be declared in `crates/capsem-gateway/src/main.rs`; unknown,
  retired, typo, or compatibility paths must return 404 without contacting the
  UDS service.
- [ ] Add/extend gateway route tests proving supported profile/plugin/VM
  routes are explicitly forwarded and unsupported paths are not forwarded. The
  test must use an unreachable UDS path so accidental fallback proxying fails.
- [ ] Extend `/vms/{vm_id}/info` to include active plugin descriptors,
  versions, modes, stages, health, and last status snapshot.
- [ ] Extend `/vms/{vm_id}/status` to include active plugin health summaries
  from in-memory runtime state only. Add an adversarial test that fails if the
  VM status path opens or reads `session.db`.
- [ ] Expose security-engine/CEL performance counters from in-memory runtime
  state: CEL compile count/errors/latency, CEL evaluation count/errors/latency,
  matched-rule count, no-match count, latency by event family/type, per-rule
  hot counters, plugin stage time, logging enqueue time, and total boundary
  time.
- [ ] Add CEL latency attribution tests proving expensive rule sets increase
  CEL counters, plugin delays increase plugin counters, and logging enqueue
  delays show separately. No counter source may require a DB read on VM status.
- [ ] Make credential broker UI state come only from VM plugin runtime status.
  Do not expose an AI broker or infer credential state from provider/rule files.
- [x] Burn `credential` as a first-party CEL/security-event root. Keep
  `credential_ref` only as shared forensic evidence on real event families and
  expose broker state only through plugin runtime status/stats.
- [x] Burn `snapshot` as a first-party CEL/security-event root unless a real
  snapshot parser/rule contract is deliberately designed later. Workspace
  snapshot operations remain MCP/tool/runtime mechanics for 1.3.
- [x] Remove `Credential` and `Snapshot` from `RuntimeSecurityEventFamily`,
  `RuntimeSecurityEventType`, logger DB event-type checks, or keep them
  explicitly documented as ledger-only emitted types. `SecurityEvent`,
  `SerializableSecurityEvent`, `SECURITY_EVENT_CEL_ROOTS`, CEL coverage tests,
  and default rules no longer expose fake credential/snapshot object roots.
  Decision: keep `credential.substitution` and `snapshot.event` as typed
  ledger-only event families because substitution and snapshot lifecycle rows
  are real forensic rows, but they are not CEL object roots. Proof:
  `cargo test -p capsem-core --lib runtime_security_event_families_mark_credential_and_snapshot_as_ledger_only -- --nocapture`;
  `cargo test -p capsem-core --lib runtime_security_event_types_keep_credential_and_snapshot_ledger_only -- --nocapture`;
  `cargo test -p capsem-core --lib security_event_cel_rejects_credential_and_snapshot_roots -- --nocapture`.
  Programmatic hunt locations:
  `crates/capsem-core/src/security_engine/mod.rs`,
  `crates/capsem-core/src/security_engine/tests.rs`,
  `crates/capsem-core/src/net/policy_config/security_rule_profile.rs`,
  `crates/capsem-core/src/net/policy_config/security_rule_profile/tests.rs`,
  `crates/capsem-core/src/net/policy_config/provider_profile.rs`,
  and `crates/capsem-logger/src/schema.rs`.
- [x] Delete `/profiles/{profile_id}/credentials/*` service and gateway routes,
  handlers, and tests. Credential state is opaque plugin runtime state exposed
  through `/vms/{vm_id}/plugins/credential_broker/status|stats`.
- [ ] Burn stale settings/defaults `settings.ai.*` and credential injection
  blocks that pretend to write host credentials into the VM. Credential
  brokering is plugin-owned and logs only brokered BLAKE3 references.
  - [x] Burn settings-to-guest materialization for brokered provider API keys,
    repository tokens, provider allow authority env vars, generated
    `.git-credentials`/`.gitconfig`, and settings-owned AI CLI config files.
    Proof:
    `cargo test -p capsem-core --lib policy_config -- --nocapture` (390 passed),
    `cargo test -p capsem-core --no-run`, and
    `cargo test -p capsem-process --no-run`.
  - [ ] Burn or reshape the remaining static `settings.ai.*` registry entries
    so settings are UI/app preferences only and provider state comes from
    profiles, rules, plugin runtime status, observed ledger evidence, and
    routing config.
  - [x] Reshape provider `[ai.*]` endpoint metadata to routing/rules/discovery
    only. Static `credential_setting_id`, provider-level `credential_ref`, and
    provider `files` are rejected; settings provider status no longer exposes
    brokered credential refs or static credential setting ids.
    Proof: `cargo test -p capsem-core --lib provider_profile -- --nocapture`
    passed 7 tests including the static metadata rejection test; full
    `cargo test -p capsem-core --lib policy_config -- --nocapture` passed 393
    tests; `pnpm -C frontend check` and `git diff --check` passed.
  - [x] Burn credential broker writeback into settings IDs. The broker stores
    secrets in the credential store/keychain, writes substitution ledger rows,
    and records provider discovery for AI observations; it no longer persists
    `credential:blake3` references into `settings.ai.*.api_key` or repository
    token setting rows.
    Proof: `cargo test -p capsem-core --lib credential_broker -- --nocapture`
    passed 7 tests; `cargo test -p capsem-core --lib brokered_ -- --nocapture`
    passed 6 focused policy_config tests; full
    `cargo test -p capsem-core --lib policy_config -- --nocapture` passed 393
    tests; `cargo test -p capsem-core --no-run`, `cargo bench -p capsem-core
    --bench security_actions --no-run`, and `git diff --check` passed.
- [x] Delete the dead `host_config` detector/writeback module and its frontend
  DTOs. This removes the setup-era path that scanned raw host API
  keys/OAuth/ADC/GitHub tokens and wrote them into settings; credential capture
  remains broker/plugin-owned, and `/settings/validate-key` stays a retired
  gateway route.
- [x] Replace legacy `[profiles.defaults.*]` parsing with `[default.<domain>]`
  rule parsing. A rule is default because `priority = "default"`, not because
  its table path says defaults twice.
  Proof: `cargo test -p capsem-core --lib security_rule_profile -- --nocapture`
  includes `legacy_profiles_defaults_authoring_is_rejected`; full
  `cargo test -p capsem-core --lib policy_config -- --nocapture` passed 391
  tests; `cargo test -p capsem-service --no-run` passed.
- [x] Burn `default_credentials` / `[default.credential]`; brokered credential
  references are evidence on real security events, not a standalone default
  traffic family.
  Proof: programmatic hunt found no `default_credentials` or `[default.credential]`
  implementation; the default-rule parser accepts only the real default
  first-party domains present in `config/profiles/code/enforcement.toml` and
  `default_provider_rules.toml`.
- [x] Delete `ProfileCredentialConfig` / `credentials.broker_enabled` parser
  support and add a rejection test for `[credentials]`.
- [x] Delete or reshape static `ProfileConfigFile.ai` / `[ai.*]` parser support
  so provider UI/status cannot be invented from metadata without allow/configured
  truth.
- [x] Delete `tool_config_sources` from static profile parsing and add a
  rejection test. Observed tool config sources belong to runtime status/security
  ledger evidence with real BLAKE3 hashes and credential refs.
  Proof: `cargo test -p capsem-core --lib tool_config_sources -- --nocapture`
  passed 4 rejection/response tests; full
  `cargo test -p capsem-core --lib policy_config -- --nocapture` passed 392
  tests; `cargo test -p capsem-core --no-run`, `pnpm -C frontend check`, and
  `git diff --check` passed.
- [ ] Validate profile parsing compiles into the new `SecurityRuleSet`/CEL rail;
  no second policy syntax or compatibility rail.
- [ ] Restore `capsem-admin` CLI package and entry point.
- [ ] Restore profile/settings `init|schema|validate|doctor` commands.
- [ ] Restore image `plan|verify|workspace|build` commands.
- [ ] Restore manifest `check|download-check|generate|sign|verify` commands.
- [ ] Restore `scripts/build-assets.sh --profile <profile>` or equivalent
  `just build-assets profile=...` typed rail.
- [ ] Restore package/bootstrap proof that `capsem-admin` is installed and
  runnable.
- [ ] Restore CI/release calls to `capsem-admin` for profile-derived assets.
- [ ] Add tests proving raw asset builds without a profile fail closed.
- [ ] Commit S1.

## S2: Runtime Profile Assets And Pins

- [x] Add core `ProfileCatalog` loader and parse the checked-in
  `config/profiles/code.toml` as the built-in real profile entry.
- [x] Replace service profile route validation/list/info/assets/skills/plugin
  profile checks with catalog-backed `code` profile lookup instead of a
  hard-coded `default` profile stub.
- [x] Make `/profiles/{profile_id}/assets/status` report the selected
  profile's current-arch kernel/initrd/rootfs contract, expected hashes, and
  present/missing state from the asset cache.
- [x] Burn live `/profiles/default` asset callers from the CLI/gateway/test
  contract. `capsem assets status|ensure` now defaults to the real `code`
  profile, accepts `--profile`, and forwards through
  `/profiles/{profile_id}/assets/...`; gateway coverage also forwards
  `/profiles/status` and `/profiles/reload` explicitly.
- [x] Remove the `ProfileConfigFile::builtin_default()` compatibility alias and
  rename built-in profile validation/tests away from "default profile"
  language. `default` remains only rule priority/visible default-rule
  vocabulary, not a profile id or fallback loader.
- [ ] Restore profile catalog/loader and remove all `default`-only profile code
  paths.
- [ ] Represent default/built-in profiles as real catalog/profile entries using
  the same loader/status/asset machinery as every other profile.
- [ ] Restore service profile inventory/status surface: profile id,
  name/description/icon, revision, catalog status, installed status,
  launchability, asset readiness, reconcile/download state, and errors.
- [ ] Restore profile list/info/status/reload/reconcile/assets-ensure routes
  needed by UI, TUI, CLI, and install checks.
- [ ] Restore profile asset download/check/refresh management in the service.
- [ ] Ensure profile asset management verifies BLAKE3 hashes and reports
  progress/errors per profile.
- [x] Enforce refresh policy at every profile/corp/asset metadata layer.
  Current contract evidence:
  `config/corp.toml` has top-level `refresh_policy`, `ProfileConfigFile`
  requires top-level profile `refresh_policy`,
  `ProfileAssetConfig` requires `assets.refresh_policy`, and `ManifestV2`
  now requires top-level `refresh_policy` with generator/docs/tests updated.
  BLAKE3 hash enforcement remains tracked by the adjacent asset verification
  items.
- [ ] Ensure VM launch fails closed on missing/corrupt profile-selected assets.
- [ ] Restore per-arch profile asset declarations with URL/hash/size.
- [ ] Restore profile-aware asset supervisor/reconcile/status/ensure.
- [ ] Ensure VM create requires and persists immutable `profile_id`.
- [ ] Restore VM profile revision/payload hash/base-asset pins.
- [ ] Make resume/fork/save fail closed on missing/corrupt/revoked/mismatched
  profile or base-asset pins.
- [ ] Expose profile id/revision/status/pins in service/gateway/client DTOs.
- [ ] Add adversarial tests for fake profiles, two profiles with different
  assets, corrupt assets, missing pins, and revoked/deprecated profiles.
- Coverage for profile-route burn slice:
  `cargo test -p capsem parse_assets -- --nocapture`;
  `cargo test -p capsem-gateway gateway_security_routes_are_explicitly_forwarded -- --nocapture`;
  `cargo test -p capsem-gateway gateway_does_not_forward_retired_profile_credential_routes -- --nocapture`;
  `cargo test -p capsem-service profile -- --nocapture`;
  `cargo test -p capsem --no-run`;
  `cargo test -p capsem-gateway --no-run`;
  `cargo test -p capsem-service --no-run`;
  `git diff --check`.
  Python API checks were attempted with `pytest` and `python3 -m pytest`, but
  this shell lacks the `pytest` module.
- Coverage for built-in profile vocabulary burn:
  `cargo test -p capsem-core --lib profile_contract -- --nocapture`;
  `cargo test -p capsem-core --lib provider_profile -- --nocapture`;
  `cargo test -p capsem-service profile -- --nocapture`;
  `cargo test -p capsem-core --no-run`.
  A non-`--lib` provider-profile filter also passed its unit assertions but
  then hit the known macOS signing wrapper while walking an unrelated
  integration binary, so the lib-only rerun is the canonical proof.
- Coverage for dead host detector burn:
  `cargo test -p capsem-core --no-run`;
  `cargo test -p capsem-gateway gateway_does_not_forward_retired_settings_utility_routes -- --nocapture`;
  `pnpm -C frontend check`.
- [ ] Commit S2.

## S3: TUI And Terminal Shell

- [ ] Restore `crates/capsem-tui` or accepted replacement.
- [ ] Restore workspace/package references for TUI.
- [ ] Restore `capsem shell` TUI launch path.
- [ ] Ensure TUI reads backend profile/session/asset contracts directly.
- [ ] Restore multi-VM/session navigation and keyboard shortcuts.
- [ ] Restore TUI VM manipulation flows: create, start, pause, resume, stop,
  save, fork, delete, and recovery where supported.
- [ ] Restore terminal attach/reconnect behavior.
- [ ] Restore profile selection/readiness/status display.
- [ ] Add regression coverage that status/readiness hotpaths do not query the
  session DB on every frame.
- [ ] Add tests for terminal shell launch, profile readiness display,
  multi-VM/session navigation, lifecycle actions, shortcuts, and corrupt/stopped
  session recovery.
- [ ] Commit S3.

## S4: Linux/KVM/EROFS/LZ4HC And Benchmarks

- [ ] Inventory Linux-team scoped commits/files.
- [ ] Restore/port Linux-team KVM/filesystem changes in scoped files.
- [ ] Preserve modern `iptables-nft` path; do not restore legacy path.
- [ ] Restore/verify EROFS/LZ4HC as accepted 1.3 runtime asset format on every
  supported architecture.
- [ ] Ensure profile/admin asset generation emits EROFS/LZ4HC for every
  supported architecture.
- [ ] Verify the built boot assets are EROFS/LZ4HC level 12 from the
  profile-selected asset chain, not from a stale benchmark artifact.
- [ ] Restore/verify multi-arch asset proof.
- [ ] Restore advanced benchmark harness/artifacts for EROFS/LZ4HC.
- [ ] Record zstd comparison evidence and decision.
- [ ] Record benchmark numbers with image format, compression, compression
  level, architecture, kernel, host OS, command line, event/workload counts,
  latency, and throughput where applicable.
- [ ] Compare benchmark numbers against the accepted 1.3 baseline and mark any
  material regression as a release blocker unless explicitly accepted by owner.
- [ ] Mark Linux-only execution proof as passed or owner-accepted handoff
  blocker.
- [ ] Commit S4.

## S5: Security Corpus And Bench Gates

- [ ] Restore detection/enforcement corpus in the new rule format.
- [ ] Restore Sigma facade/import/export tests for detection rules.
- [ ] Restore pack/corpus compile and backtest commands through `capsem-admin`
  or the accepted typed admin rail.
- [ ] Restore security-event benchmarks for HTTP, DNS, MCP, model, process, and
  file events.
- [ ] Add regression tests proving old policy-v2/domain/MCP decision rails stay
  absent.
- [ ] Commit S5.

## S6: Docs, Changelog, And Verification

- [ ] Restore current-truth profile/admin command docs.
- [ ] Restore profile assets/catalog docs against the current contract.
- [ ] Restore benchmark docs/page with current 1.3 numbers.
- [ ] Update changelog.
- [ ] Run focused tests for S1-S5.
- [ ] Run smoke.
- [ ] Run install cycle.
- [ ] Boot a profile-selected VM from restored EROFS/LZ4HC assets.
- [ ] Run `capsem-doctor` inside the VM and require green output.
- [ ] Prove file snapshot create/list/restore through the accepted runtime path.
- [ ] Run UI and TUI sanity.
- [ ] Run benchmark gate or record Linux handoff.
- [ ] Update benchmark docs/page with current EROFS/LZ4HC numbers and note any
  Linux handoff explicitly.
- [ ] Commit S6.
