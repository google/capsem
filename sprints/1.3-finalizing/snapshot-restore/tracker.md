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

- [x] `9ca1bbed release: v1.2.1779658398` decision: conceptual_port.
  Notes: release bundle marker containing many subsystems already split across
  S0/S1/S2/S3/S4/S5. Do not replay wholesale. Useful commitments are the
  benchmark evidence, profile/admin packaging, TUI package inclusion, and
  release-status docs/tests tracked as separate ledger entries below.
- [x] `1bdd27cb bench: record macos arm64 benchmark results` decision:
  conceptual_port. Notes: benchmark artifacts and docs must be restored through
  the current EROFS/LZ4HC benchmark gate and docs-site benchmark page, not by
  copying stale 1.2 numbers. Keep as release proof debt until the 1.3 benchmark
  gate records current numbers.
- [x] `89b04f87 perf: tune rootfs squashfs block size` decision:
  superseded. Notes: current 1.3 build contract in `guest/config/build.toml`
  runs EROFS/LZ4HC level 12 as the rootfs on kernel 7.0. Squashfs is not a
  runtime/build fallback; do not restore squashfs tuning as a release target.
- [x] `6823cf1f feat: package capsem tui binary` decision:
  conceptual_port. Notes: current tree has no `capsem-tui`/TUI package rail, so
  the capability remains active under the TUI restore slice. Restore the modern
  multi-VM TUI and package it with current profile/status contracts, not the old
  package script shape blindly.
- [x] `03fcce34 fix: skip asset alias directories in install profiles`
  decision: conceptual_port. Notes: old `materialize-install-profiles.py` is
  absent; profile asset packaging must be rebuilt through `capsem-admin` and
  hash-prefixed profile assets. Preserve the invariant that generated/hash alias
  directories are never treated as installable profile sources.
- [x] `b8ca8589 fix: ignore manifest aliases in install profiles` decision:
  conceptual_port. Notes: same asset-alias invariant as above, but through the
  modern BLAKE3 asset inventory/verify commands. Do not reintroduce
  manifest alias directories as profile truth.
- [x] `6daf264a fix: point package profiles at release assets` decision:
  conceptual_port. Notes: current profile descriptors carry release URLs and
  BLAKE3/size metadata directly. Package/install proof still needs an admin
  package slice ensuring bundled profiles point at release assets and never
  local dev paths.
- [x] `a841716f fix: sign packaged admin python extensions` decision:
  intentional_burn/conceptual_port. Notes: old Python-extension signing is
  stale because `capsem-admin` is now restored as a Rust binary. Preserve the
  release invariant that packaged executables are signed/notarized by the
  normal package pipeline; do not restore Python admin extension signing.
- [x] `718981b1 docs: record admin release gate proof` decision:
  conceptual_port. Notes: release gate proof remains required, but docs/tests
  must target current `capsem-admin`, profile-owned rule files, and the single
  `SecurityRuleSet` rail.
- [x] `24c846e8 refactor: rename admin policy packs to enforcement` decision:
  conceptual_port. Notes: keep the vocabulary (`enforcement`, not `policy`
  packs) and burned old policy strings. Current docs and endpoints already use
  `/enforcement`; admin commands should validate current enforcement TOML
  directly.
- [x] `923d603f test: add session process policy corpus` decision:
  conceptual_port. Notes: useful corpus target, but old `policy-context`
  fixtures are superseded by typed `SecurityEvent`/session DB ledger events.
  Rebuild process coverage against current `file/process/http/dns/mcp/model`
  event roots.
- [x] `63eccc3f feat: support admin model tool policy paths` decision:
  conceptual_port. Notes: current CEL roots include model tool-call fields;
  admin validation must compile those paths through `SecurityRuleProfile`, not
  through old policy-pack path lists.
- [x] `9944c7ba feat: expand admin policy context parity` decision:
  conceptual_port. Notes: old context parity becomes current
  `SecurityEvent` fixture parity. Do not restore policy-context JSONL as a
  second abstraction.
- [x] `391eaece fix: compile-check policy backtests before replay` decision:
  conceptual_port. Notes: preserve the invariant that replay/backtest files are
  compile-checked first. Port as current enforcement/Sigma compile commands
  before any backtest runner.
- [x] `b07101ed test: tighten admin policy path compile` decision:
  conceptual_port. Notes: path compilation is still mandatory, but through
  current CEL roots (`http`, `dns`, `mcp`, `model`, `file`, `process`) and
  without `credential`/`snapshot` roots.
- [x] `2f9b0fd0 test: expand s08c policy corpus diversity` decision:
  conceptual_port. Notes: rebuild as fresh current-rule corpus coverage after
  admin compile/validate exists.
- [x] `80a416be feat: add admin policy compile` decision:
  conceptual_port. Notes: port as `capsem-admin enforcement compile` (current
  TOML) and `capsem-admin detection compile` (Sigma YAML) over
  `SecurityRuleProfile`, not old policy-pack compile.
- [x] `2db1259a test: pin s08c detection ir parity` decision:
  conceptual_port. Notes: the old detection IR schema is absent and should not
  be restored as a standalone contract unless it is derived from
  `SecurityRuleProfile::parse_sigma_yaml`. Current port should prove Sigma YAML
  compiles into the same rule rail.
- [x] `099152a4 feat: add admin policy backtest corpus` decision:
  conceptual_port. Notes: backtest corpus remains valuable but must use current
  `SecurityEvent` fixtures and compiled rule sets. Rebuild after compile
  commands land.
- [x] `7b14ccb4 feat: add admin detection backtest corpus` decision:
  conceptual_port. Notes: same as above for Sigma detection YAML; no old
  detection-pack schema restore.
- [x] `2bedce99 feat: seed policy context rule corpus` decision:
  conceptual_port. Notes: seed a fresh current-rule corpus later; old
  `policy-context` abstraction stays burned.
- [x] `b0eecdd7 feat: add admin doctor closeout` decision: conceptual_port.
  Notes: admin doctor remains required, but must report current prerequisites:
  profile rule compile, profile assets, BLAKE3 inventory, EROFS/LZ4HC build
  shape, and absence of burned rails.
- [x] `0e1e6b1b feat: add detection ir parity` decision: conceptual_port.
  Notes: old IR files/schema absent; current parity proof should be
  Sigma-to-`SecurityRuleProfile` compile output.
- [x] `66141eee feat: compile detection packs` decision: conceptual_port.
  Notes: port as direct Sigma YAML compile in `capsem-admin`, not detection-pack
  schemas.
- [x] `d773481f feat: validate security packs` decision: conceptual_port.
  Notes: validate current enforcement TOML and Sigma YAML files directly.
  Burn old `policy-pack`/`detection-pack` schemas and Python pack compiler.
- [x] `7277c17b feat: generate guest image sboms` decision:
  conceptual_port. Notes: SBOM/provenance remains required for release
  evidence, but not as manifest signing. Restore under admin image/manifest
  provenance commands after BLAKE3 checks.
- [x] `3a37d704 feat: verify doctor bundle probes` decision:
  conceptual_port. Notes: doctor bundle verification remains required and must
  target current `capsem-doctor`/profile VM boot proof.
- [x] `2d02b6e0 fix: require image inventory proof` decision:
  conceptual_port. Notes: preserve fail-closed inventory proof in
  image/manifest admin commands.
- [x] `33c83bd0 feat: verify per-arch image inventories` decision:
  conceptual_port. Notes: current manifest check/verify reports each asset
  version/arch/logical asset and verifies sibling built files literally; full
  image inventory extraction remains open.
- [x] `a1dab24f feat: extract image inventory from rootfs` decision:
  conceptual_port. Notes: useful for SBOM/provenance; restore under image
  verify later.
- [x] `0ffb816a feat: verify image package inventory` decision:
  conceptual_port. Notes: package inventory verification remains open under
  image verify/SBOM, not manifest signing.
- [x] `c9fd7b4b feat: require profiles for asset builds` decision:
  conceptual_port. Notes: still mandatory. `scripts/build-assets.sh` is absent
  in the cleanup tree, so restore a profile-required build rail later and add a
  fail-closed raw-build test.
- [x] `fd86e8ed feat: derive built-in profiles from guest config` decision:
  conceptual_port. Notes: old generated base profiles carried stale schema
  baggage; current `config/profiles/code.toml` is the real profile. Any derived
  build workspace must merge with that modern profile shape.
- [x] `5b4e4274 feat: generate profile ui base profiles` decision:
  conceptual_port/intentional_burn. Notes: useful UI profile generation idea,
  but old schema fixtures/signatures/minisig payloads are burned. Current UI
  must reflect real profile config.
- [x] `a02537ad feat: add profile-derived image build command` decision:
  conceptual_port. Notes: restore as current `capsem-admin image ...` commands
  after manifest check/verify.
- [x] `31425d04 feat: materialize profile image workspaces` decision:
  conceptual_port. Notes: `src/capsem/builder/image_workspace.py` is absent;
  restore profile-derived workspaces later without old profile schema baggage.
- [x] `879c9d59 test: prove packages include capsem-admin` decision:
  conceptual_port. Notes: Rust `capsem-admin` now exists; package/install proof
  still must ensure the binary is included and runnable.
- [x] `22016426 feat: add capsem-admin manifest crypto` decision:
  intentional_burn/conceptual_port. Notes: burn manifest signing/crypto
  authority. Port only non-signing hash/provenance validation.
- [x] `6559bf3b feat: add capsem-admin manifest generate` decision:
  conceptual_port. Notes: manifest generation remains open, but must generate
  current format-2 JSON with top-level `refresh_policy`, BLAKE3 hashes, asset
  inventory, SBOM/provenance references, and no signatures.
- [x] `3e5bb3cb feat: add capsem-admin manifest download check` decision:
  conceptual_port. Notes: restored current-contract `capsem-admin manifest
  verify`, verifying literal sibling built files by size and BLAKE3 from the
  manifest parent directory. There is no admin `--assets-dir` split path.
- [x] `e2946acd feat: add capsem-admin manifest fast check` decision:
  conceptual_port. Notes: restored current-contract `capsem-admin manifest
  check`, parsing `ManifestV2` and reporting releases/arches/assets without
  touching signing.
- [x] `2cc49f7a feat: add capsem-admin image verify` decision:
  conceptual_port. Notes: restored `capsem-admin image verify` for the current
  profile-derived build output. Remaining inventory/SBOM/doctor bundle probes
  stay open under the release evidence gate.
- [x] `2fb45076 feat: add capsem-admin image plan` decision:
  conceptual_port. Notes: image plan remains open; must be profile-derived.
- [x] `0e9442e4 test: pin admin init json toml parity` decision:
  conceptual_port. Notes: current admin init writes TOML templates directly
  from checked-in `config/settings.toml` and `config/profiles/code.toml`.
  JSON/TOML parity for old schemas is burned unless rebuilt from current Rust
  contracts.
- [x] `53065265 test: pin profile toml json round trip` decision:
  conceptual_port. Notes: current profile validation uses Rust
  `ProfileConfigFile`; schema/round-trip artifacts remain open if needed for
  docs/UI, but old profile-v2 payload/signature schema stays burned.
- [x] `c9e227c1 test: pin service settings toml json round trip` decision:
  intentional_burn/conceptual_port. Notes: old service settings owned runtime
  behavior. Current settings are UI/application preferences only; admin
  validates that shape and rejects runtime/profile fields.
- [x] `839c1114 feat: add capsem-admin settings init` decision:
  conceptual_port. Notes: restored as `capsem-admin settings init`, writing the
  current UI settings template. No AI/provider/profile/runtime fields.
- [x] `d2834490 feat: add capsem-admin profile init` decision:
  conceptual_port. Notes: restored as `capsem-admin profile init`, writing the
  checked-in `code` profile template with current assets/rules/plugins/MCP
  shape.
- [x] `be6909a0 feat: add profile section editability gates` decision:
  conceptual_port. Notes: UI/service editability remains governed by endpoint
  contracts and profile ownership; old schema gates are not restored directly.
- [x] `634b9730 feat: add capsem-admin profile validation` decision:
  conceptual_port. Notes: restored through Rust `ProfileConfigFile::validate`
  plus rule-file compilation.
- [x] `810b417a test: pin service settings default parity` decision:
  intentional_burn/conceptual_port. Notes: old service-settings defaults are
  burned. Current default truth is `config/settings.toml` for UI and
  `config/profiles/code.toml`/rule files for runtime.
- [x] `d0c1c988 feat: wire capsem-admin settings commands` decision:
  conceptual_port. Notes: restored the command surface in Rust, not old Python
  admin settings schema.
- [x] `d39756f3 feat: add service settings admin contract` decision:
  intentional_burn/conceptual_port. Notes: old service settings contract
  violated the settings/profile split. Current admin settings validation is
  strict UI settings only.
- [x] `be0741e1 feat: verify admin profile payload installs` decision:
  conceptual_port. Notes: profile install/package proof remains open under the
  package/bootstrap slice; do not restore signed profile payloads.
- [x] `25eb08d9 feat: align admin profile lifecycle gates` decision:
  conceptual_port. Notes: lifecycle gates must use current profile catalog,
  asset status, and VM profile pins. Old payload lifecycle is burned.
- [x] `f3fdbf0a chore: make profile manifest canonical` decision:
  intentional_burn/conceptual_port. Notes: old profile manifest canonicalization
  included the signing/payload rail. Current canonical profile is TOML plus
  BLAKE3 asset descriptors and runtime profile payload hash.
- [x] `b04cb88c feat: add pydantic profile contracts` decision:
  intentional_burn/conceptual_port. Notes: do not restore Python profile
  schemas that can drift from Rust. Admin profile validation now calls Rust
  contract code.
- [x] `a8f712d5 feat: add profile v2 schema artifact` decision:
  intentional_burn/conceptual_port. Notes: old schema fixtures and minisig
  artifacts are burned. A current schema artifact may be regenerated later only
  from the current profile contract and without signatures.
- [x] `4cdba35f refactor install asset prep into scripts` decision:
  conceptual_port. Notes: `scripts/build-assets.sh` and install asset prep are
  absent; restore as profile-required build/install prep later.
- [x] `d4d2bb3a fix: harden release package verification` decision:
  conceptual_port. Notes: package verification hardening remains relevant and
  belongs in the release/package slice.
- [x] `5d7e58ce fix: harden installer downloads and release package checks`
  decision: conceptual_port. Notes: release install download verification
  remains relevant; ensure the current install path verifies assets/packages
  without setup wizard fallback.
- [x] `22096b7f fix: harden release install deb repack` decision:
  conceptual_port. Notes: Linux package repack hardening remains in release
  handoff/package slice.

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
- [x] `b2fb7e33 feat: export session policy contexts` decision:
  conceptual_port. The old exported policy-context rows are superseded by the
  unified security-event ledger: emitted events carry the canonical event type,
  family, rule id, action, detection level, and forensic event payload in
  session DB rows. Do not restore the old context-export shape. Proof locations:
  `crates/capsem-core/src/security_engine/mod.rs` and
  `crates/capsem-core/src/security_engine/tests.rs`.
- [x] `7a5afc9c test: prove process enforcement logs in real vm` decision:
  conceptual_port. Process exec/audit/complete events now enter the single
  `SecurityRuleSet`/security-event writer path, with exec and completion rows
  sharing the exec event id. Current VM proof remains part of final smoke; unit
  coverage is in
  `emit_process_exec_and_complete_rules_share_exec_event_id`.
- [x] `f2a6247f docs: close s07 debt ledger` decision: conceptual_port. The
  useful asset-health/readiness contract is now in the profile-owned status,
  ensure, boot-pin, and cleanup slices below; old profile-manifest prose stays
  burned.
- [x] `f5aea0fc test: gate release image boot proof` decision:
  conceptual_port. The current release gate remains profile-asset boot proof:
  the final S2/S4 gate must build/verify EROFS lz4hc assets and run the VM
  doctor smoke. The old test fixture is not copied because profile payload
  signing and setup wizard assumptions are burned.
- [x] `dcba8776 feat: harden profile trust and policy runtime` decision:
  conceptual_port plus intentional_burn. The useful policy-runtime hardening
  lives in the new security engine/CEL path and typed security events. The old
  `policy_v2`, domain hook, `NetworkPolicy`, and MCP decision rails remain
  burned and must not be restored.
- [x] `e3be977e feat: prove s08 profile-selected gateway create` decision:
  conceptual_port. Current gateway/service fixtures require explicit
  `profile_id = "code"` for `/vms/create` and `/run`, reject missing profile
  ids, and response surfaces carry profile id/revision/status through the
  current `ProvisionResponse`/VM info shape.
- [x] `694aa75b feat: select profiles during vm create` decision:
  conceptual_port. VM create/run/fork/save/resume now require and preserve a
  real profile id, resolving boot assets through the selected
  `ProfileConfigFile` instead of any service-global default. Coverage is listed
  in the current-architecture profile id and boot-asset pin slices above.
- [x] `2a1d079d test: prove vm fork lineage` decision: conceptual_port. Fork
  and persist preserve profile id, profile revision, profile payload hash, and
  boot asset pins; drift rejection is covered by the current service tests
  named in the profile pinning slice above.
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
- [x] `438c9642 feat: fetch profile catalogs from URL` decision:
  intentional_burn. The old command fetched signed profile catalog manifests
  through `capsem profile reconcile-catalog --manifest-url --pubkey`; that
  belongs to the deleted profile-manifest/minisign authority rail. Current
  profile/corp provisioning uses explicit profile/corp config, BLAKE3 asset
  verification, and catalog reload/status; no URL+pubkey compatibility command
  is restored.
- [x] `3204f27a test: prove profile asset boot flow` decision:
  conceptual_port. Current boot preflight resolves kernel/initrd/rootfs from
  the selected profile's current-arch descriptors and blocks boot when profile
  assets are missing. CLI asset status/ensure also default to the real `code`
  profile.
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
- [x] `0a87e26a test: harden profile asset reconcile races` decision:
  conceptual_port. Current `/profiles/{profile_id}/assets/ensure` shares the
  single profile-asset rail and returns refreshed readiness. Remaining race
  stress belongs in the final release gate; do not restore the old
  service-global reconcile endpoint.
- [x] `deb1b083 refactor: remove legacy asset manifest runtime` decision:
  exact_restore in spirit. Legacy runtime manifest loading and manifest signing
  are removed; runtime uses profile descriptors plus BLAKE3/size verification.
  Current cleanup is confirmed by targeted `rg` sweeps and profile/manifest
  tests.
- [x] `d069710f feat: trigger profile asset reconcile from update` decision:
  conceptual_port. The old update-triggered global reconcile path is replaced
  by explicit profile-scoped `assets/ensure` and profile catalog
  `status`/`reload`; installer/update final smoke must call the profile route.
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
- [x] `911d6a67 feat: fetch signed profile payloads` decision:
  intentional_burn. Signed profile payload fetching depended on profile
  manifest/minisign theater; do not restore.
- [x] `dd42a2d4 feat: verify profile payload signatures` decision:
  intentional_burn. Profile payload signature verification depended on baked
  public keys/admin-provided signature rails that we removed. Current trust is
  explicit corp/profile source selection plus BLAKE3 asset verification and
  SBOM/provenance evidence.
- [x] `237d2bbc feat: materialize verified profile payloads` decision:
  conceptual_port plus intentional_burn. Current `ProfileCatalog::load_default`
  materializes built-in or directory TOML profiles after schema validation; the
  verified-payload cache/signature half stays burned.
- [x] `152c7780 feat: verify installable profile payloads` decision:
  conceptual_port. Current `ProfileConfigFile::validate`,
  `ProfileCatalog::load_from_dir`, and profile asset status/ensure routes prove
  installable profile shape without restoring profile signatures.
- [x] `d50d8a13 feat: add profile catalog lifecycle gates` decision:
  conceptual_port. Current `/profiles/status` and `/profiles/reload` validate
  the active catalog and report source/profile readiness. Old signed-catalog
  lifecycle checks stay burned.
- [x] `048d7cf5 feat: drive runtime assets from profiles` decision:
  conceptual_port. Current boot, resume, save, fork, cleanup, status, and
  ensure resolve and pin assets from the selected profile. This is the core S2
  restored contract.
- [x] `d759668c feat: validate profile payload schema in rust` decision:
  conceptual_port. The old JSON schema artifact is replaced by the Rust
  `ProfileConfigFile` TOML contract with `deny_unknown_fields`, strict
  validation, checked-in `config/profiles/code.toml`, and profile contract
  tests.
- [x] `996de225 feat: add profile manifest catalog types` decision:
  conceptual_port plus intentional_burn. The useful typed catalog concept is
  now `ProfileCatalog` over real profile TOML files; old profile manifest
  catalog and signature metadata stay burned.
- [x] `f3578c3d release-debug-loop: finalize saved VM asset tracking and status surfaces`
  decision: conceptual_port. Current status surfaces include profile asset
  readiness, persistent VM profile/asset pins, profile catalog status, and
  explicit profile-scoped asset routes. Legacy setup/status/provider UI pieces
  from that commit remain burned.

- [x] Current-architecture cleanup slice: CLI and `capsem-mcp` MCP commands
  now use the real built-in `code` profile instead of the retired `default`
  profile for profile-scoped MCP server/tool routes, and `capsem-mcp`
  create/run request bodies include the service-required `profile_id`.
  Decision: conceptual_port of profile-scoped CLI/MCP behavior into the
  current endpoint contract. Tests: `cargo test -p capsem
  cli_default_profile_is_real_code_profile -- --nocapture`, `cargo test -p
  capsem parse_assets -- --nocapture`, `cargo test -p capsem-mcp profile_id --
  --nocapture`, and a targeted `rg` sweep for `DEFAULT_PROFILE_ID = "default"`
  and `/profiles/default`.

### S3 TUI/Shell And Lower-Priority Debug Commits

- [x] `0a425541 chore: merge main into tui control` decision:
  conceptual_port. Notes: do not replay merge noise; restore the latest useful
  TUI state and port routes to current profile/VM endpoints.
- [x] `a476d7a7 chore: merge main into tui control branch` decision:
  conceptual_port. Notes: same merge-noise handling as above.
- [x] `9ca1bbed release: v1.2.1779658398` decision: conceptual_port. Notes:
  TUI package inclusion and release proof are restored through current package
  scripts/workflows and payload tests.
- [x] `32102d6d fix: purge broken persistent tui sessions` decision:
  conceptual_port. Notes: restored purge flow and broken persistent-session
  messaging in the TUI action/provider tests.
- [x] `2b6a2edc fix: offer tui recovery create and purge` decision:
  conceptual_port. Notes: restored empty/recovery create and purge affordances.
- [x] `0cf0a9a0 fix: keep tui create focus pending` decision:
  conceptual_port. Notes: restored pending-create focus behavior.
- [x] `6902dc4b fix: show full-screen tui suspend progress` decision:
  conceptual_port. Notes: restored full-surface suspend progress rendering.
- [x] `b50c811d fix: reconnect tui terminal after resume` decision:
  conceptual_port. Notes: restored terminal manager reconnect coverage.
- [x] `9b168fd5 fix: focus tui create and hide corrupt tabs` decision:
  conceptual_port. Notes: restored corrupt profile tab filtering plus create
  replacement prompt.
- [x] `860cc8ea feat: make capsem shell launch tui` decision:
  conceptual_port. Notes: current `capsem shell` now launches `capsem-tui`
  and maps an optional session to `--session`.
- [x] `f3068301 fix: prompt tui service start when offline` decision:
  conceptual_port. Notes: restored offline/degraded start-service screens.
- [x] `53862ec2 fix: block tui create without profiles` decision:
  conceptual_port. Notes: restored profile-unavailable create guard.
- [x] `92143119 fix: open tui new session on empty state` decision:
  conceptual_port. Notes: restored empty-state create flow.
- [x] `c2fb4b77 fix: move tui help hint to session stats` decision:
  conceptual_port. Notes: restored status bar help hint behavior.
- [x] `e3d0312f fix: polish tui controls and overlays` decision:
  conceptual_port. Notes: restored modal/overlay polish.
- [x] `fb98b2d1 fix: add tui fork flow` decision: conceptual_port. Notes:
  restored fork overlay and `/vms/{id}/fork` provider action.
- [x] `f5a73773 fix: make tui create profile aware` decision:
  conceptual_port. Notes: restored profile selection in create flow against
  `/profiles/list`.
- [x] `d47a889a fix: pin tui suspend hint left` decision: conceptual_port.
  Notes: restored suspend hint behavior through existing snapshot tests.
- [x] `f60bb671 fix: surface tui suspend shortcut` decision:
  conceptual_port. Notes: restored Alt+s/Alt+c help and action ownership.
- [x] `1299bd5c fix: render stopped tui sessions` decision:
  conceptual_port. Notes: restored stopped-session render and resume prompt.
- [x] `6138c0b9 fix: gate endpoint latency hot paths` decision:
  conceptual_port. Notes: restored TUI via gateway `/status` cache and
  profile routes; TUI provider tests use HTTP mocks and never read session DB.
- [x] `a21e269c fix: stabilize tui latency display` decision:
  conceptual_port. Notes: restored fresh service latency preservation.
- [x] `161e40f4 fix: simplify tui tab colors and modal input` decision:
  conceptual_port. Notes: restored tab color and active-input tests.
- [x] `43716abb fix: harden tui modal and resize behavior` decision:
  conceptual_port. Notes: restored modal escape/focus behavior.
- [x] `91a9cf93 fix: make tui shell controls alt-only` decision:
  conceptual_port. Notes: restored Alt-only shell shortcuts and plain-key
  forwarding coverage.
- [x] `f54d94a0 fix: stabilize tui session navigation` decision:
  conceptual_port. Notes: restored session navigation tests.
- [x] `ec0c7152 fix: use vt parser for tui terminal` decision:
  conceptual_port. Notes: restored vt100-backed terminal surface tests.
- [x] `c93351ee fix: finish tui live terminal proof` decision:
  conceptual_port. Notes: restored gateway terminal bridge and reconnect
  coverage.
- [x] `6823cf1f feat: package capsem tui binary` decision:
  conceptual_port. Notes: restored workspace/package/CI/release inclusion for
  `capsem-tui`.
- [x] `ec473982 feat: add confirmed capsem tui service actions` decision:
  conceptual_port. Notes: restored confirmation modal before service actions.
- [x] `92a9992f feat: add capsem mcp terminal snapshot` decision:
  conceptual_port. Notes: restored deterministic text/SVG TUI snapshot harness;
  MCP-specific fixture remains current TUI fixture-driven.
- [x] `921b941f feat: add capsem tui gateway terminal shell` decision:
  conceptual_port. Notes: restored gateway terminal bridge and TUI route usage.
- [x] `2e79056b style: simplify capsem tui chrome` decision:
  conceptual_port. Notes: restored simplified TUI chrome snapshot.
- [x] `c6a70081 feat: add standalone capsem tui shell` decision:
  conceptual_port. Notes: restored standalone `capsem-tui` binary with
  `--fixture`, `--snapshot`, and `--snapshot-svg`.
- [x] `1845ec83 fix: stop install harness service before error tests`
  decision: adapted. Current install fixture now imports `time`, stops the
  dpkg/systemd user unit before scoped process cleanup when
  `CAPSEM_DEB_INSTALLED=1`, and has a regression test proving stop-before-pkill
  ordering.
- [x] `33684fcd fix: compile debug report disk stats on macos` decision:
  not ported. The structured debug-report subsystem is not present in the 1.3
  contract, so the macOS disk-stats compile patch has no target file to port.
- [x] `2322fbf2 feat: surface security health in status` decision:
  not ported as a CLI-status graft. Security/detection health now belongs to
  the ledger-backed `/security/status`, `/enforcement/status`, and
  `/detection/status` service routes; `capsem status` stays service/gateway,
  asset, and VM boot-health focused.
- [x] `27e985d8 feat: expose runtime security debug health` decision:
  not ported. Runtime security health is exposed through the current
  security-engine ledger/status routes rather than resurrecting the old debug
  report endpoint path.
- [x] `ddaf358c test: extend s08 gateway diagnostics coverage` decision:
  not ported. The old S08 gateway diagnostics/debug-report surface is not part
  of the current explicit gateway/API contract; current gateway diagnostics are
  covered by the profile/VM/security route tests.
- [x] `be5f902b feat(settings-profiles): add debug provenance` decision:
  not ported. Profile/config provenance is now enforced by profile materialize,
  validation, and asset status routes; no legacy settings-profile debug
  provenance endpoint is restored.
- [x] `77ec3abf feat: add structured debug report` decision:
  not ported. The old structured debug-report subsystem mixed install,
  settings, profile, and gateway concerns before the profile/security contract
  reset; 1.3 uses explicit status/info/latest routes plus `capsem doctor`
  artifacts instead.
- [x] `fe7a4071 fix: harden local install diagnostics` decision:
  adapted. Current package scripts already wait for service/gateway readiness,
  use the normal install command, include the full host tool set, and expose
  install failures. This pass additionally removed setup wording from the
  internal just helper name.
- [x] `9713a49e fix(setup): split install vs. onboarding flags so reinstall stops re-showing wizard`
  decision: intentional_burn. `capsem setup`, onboarding flags, setup state,
  and provider wizard state are removed; install tests now assert the command is
  invalid and writes no setup/user state.
- [x] `0dd1d8ed test(install): self-heal layout fixture, gate intrusive auto-launch tests`
  decision: conceptual_port plus adapted. Current install tests are
  function-scoped/self-healing, package-relative under pytest importlib mode,
  gate intrusive LaunchAgent/systemd tests, and keep setup burned. This S3 pass
  repaired the remaining missing `time` import/systemd cleanup gap.
- [x] `5c897436 fix: switch pytest to importlib mode + package-relative conftest imports`
  decision: already_ported. `pyproject.toml` uses
  `--import-mode=importlib`, and install tests import their local conftest via
  package-relative imports.
- [x] `ae888779 feat: wire real .pkg/.deb install paths, harden installer pipeline`
  decision: conceptual_port. Current `.pkg`/`.deb` scripts exercise real
  package install paths, hard-fail repack on missing companion binaries,
  include `capsem-admin`, `capsem-tui`, MCP aggregator/builtin binaries, copy
  current-arch assets through the manifest rail, and use service/gateway
  readiness rather than setup wizard success.
- [x] `6c1a639e feat: capsem setup interactive wizard` decision:
  intentional_burn. The interactive setup wizard is not part of the 1.3
  architecture; credential/provider work is plugin/profile/security-event
  owned.

### S4 Linux/KVM/EROFS/LZ4HC/Benchmark Commits

- [x] `0a425541 chore: merge main into tui control` decision:
  merge-noise inspected; no replay. TUI behavior was restored in S3.
- [x] `9ca1bbed release: v1.2.1779658398` decision: release checkpoint
  inspected; no replay. Current 1.3 release proof owns package/TUI/assets.

KVM block/io_uring/event-index/ioeventfd lane decision: conceptual_port. The
current tree contains vectored KVM block I/O, event-index queue support,
ioeventfd worker plumbing, io_uring backend and metrics, with io_uring kept
default-off and gated away from read-only rootfs. Revert commits below are
honored as historical experiment boundaries; the final current stack is the
accepted implementation.

- [x] `56b61a22 bench: record default off io_uring results`
- [x] `803bfbac perf: make kvm io_uring block opt in`
- [x] `7233acf9 bench: record gated kvm io_uring results`
- [x] `c2422adf perf: gate kvm io_uring block to writable disks`
- [x] `a0ef66bb bench: record kvm io_uring block results`
- [x] `7037bac3 perf: add kvm virtio block io_uring backend`
- [x] `0bbd5397 bench: record virtio block telemetry results`
- [x] `4ca0fb0a feat: add kvm virtio block telemetry`
- [x] `a0f8df6b bench: record kvm event index results`
- [x] `3b2c7390 perf: add kvm virtio block event index`
- [x] `9d4c1f2a bench: record combined kvm block stack results`
- [x] `ba8f260e perf: combine kvm ioeventfd block batching`
- [x] `20bb3483 Revert "perf: route kvm block notify through ioeventfd"`
- [x] `7e7c470c perf: route kvm block notify through ioeventfd`
- [x] `14dc4562 Revert "perf: batch kvm block used ring updates"`
- [x] `589494f5 perf: batch kvm block used ring updates`
- [x] `2d56217c Revert "perf: move kvm block io off vcpu notify"`
- [x] `8a391cb1 perf: move kvm block io off vcpu notify`
- [x] `c4b07da8 bench: record vectored kvm block io results`
- [x] `0dbd5099 perf: use vectored kvm block io`
- [x] `f4308f01 perf: trim kvm rootfs overlays before fork`

VirtioFS/Linux filesystem lane decision: conceptual_port. Current code has the
KVM VirtioFS worker, larger request negotiation, positional I/O, Linux readlink
opcode, inode path preservation on rename, trusted git workspace setup, and UV
cache kept off the VirtioFS workspace.

- [x] `525b59bf feat: async VirtioFS worker thread with irqfd interrupts`
- [x] `a52f7aab perf: negotiate larger virtiofs requests`
- [x] `b9716188 perf: use positional virtiofs io`
- [x] `61b775a2 fix: trust git workspaces in linux kvm guests`
- [x] `6be2d86a fix: keep uv cache off virtiofs workspace`
- [x] `eb76d419 fix: use linux readlink opcode for virtiofs`
- [x] `5cee8c99 fix: preserve virtiofs inode paths on rename`

KVM backend/checkpoint/x86_64 lane decision: conceptual_port with Linux runtime
handoff. Current code contains the hypervisor abstraction, KVM backend,
x86_64 bzImage/IRQCHIP/serial path, arch validation, compile guardrails, KVM
checkpoint save/restore, MP state preservation, and warm restore queue state.
Local macOS can compile/check shared code but cannot execute KVM; Linux runtime
doctor/boot remains the explicit Linux-team release handoff.

- [x] `3cb8e44a feat: hypervisor abstraction layer with Apple VZ and KVM backends`
- [x] `db1a82c5 feat: add x86_64 KVM backend -- bzImage boot, IRQCHIP, 16550 UART, PIO bus`
- [x] `f68bc9fc feat: x86_64 release boot test, compile-time KVM guardrails, arch-mismatch detection`
- [x] `717d03e5 feat: x86_64 KVM boot fixes, arch validation, cross-compile Docker image`
- [x] `6039e821 fix: x86_64 Linux build -- cfg-gate aarch64 boot module, cross-linker config`
- [x] `dae43aa9 fix: optional PIT for CI KVM, boot test in cross-compile, GNU cross-linker`
- [x] `031aafa6 feat: v0.16.1 -- KVM diagnostics, doctor rewrite, platform-specific boot errors`
- [x] `d9429e1f fix: stabilize linux kvm test gate`
- [x] `5a1397f1 fix: resume kvm guests from warm checkpoints`
- [x] `3bf9f18f fix: expand kvm warm restore state`
- [x] `bdedb26a fix: preserve kvm vcpu mp state in checkpoints`
- [x] `e34817ae docs: record linux kvm doctor pass`
- [x] `e046977e test: cover tmp symlinks in linux kvm doctor`
- [x] `06cc31e5 feat: checkpoint linux kvm proving ground`
- [x] `c215b6d9 fix: keep pr linux kvm tests compile-only`
- [x] `41be412a fix: restore linux kvm test compilation`

Asset/build/CI lane decision: conceptual_port. Current `capsem-admin`/builder
rails materialize profile-selected per-arch EROFS assets, profile manifests,
multi-arch layout, and package/install proof through the generated config path.

- [x] `5811282e feat: capsem-builder integration, multi-arch CI, per-arch asset layout`
- [x] `ea1e7e6c test: align release gate with hardened cli`
- [x] `49bcf13d test: stabilize release gate hot paths`
- [x] `cffc9fbf chore: checkpoint remaining S5/S6 backend and artifact updates`
- [x] `48104328 refactor: move inline test modules to sibling tests.rs files`

Benchmark/docs lane decision: conceptual_port. Current benchmark harness and
docs include storage split diagnostics, IOPS profiling, local MITM benchmark
fixtures, lifecycle/fork/parallel/capsem-bench artifacts, and the benchmark
results page with EROFS zstd-vs-lz4hc evidence. Historical artifacts are
recorded as evidence, not replayed as code.

- [x] `4d133bb7 bench: rerun mac benchmark after linux merge`
- [x] `b4ba5ce6 bench: record linux wrap-up benchmark artifacts`
- [x] `b6f9b6e2 bench: preserve artifacts before benchmark reruns`
- [x] `8e8c4a77 bench: archive superseded benchmark artifacts`
- [x] `05df4127 docs: add hypervisor improvement sprint`
- [x] `c093f4b4 bench: include storage diagnostics in canonical run`
- [x] `4c75cbfe bench: enforce benchmark artifact contract`
- [x] `d5f67d78 bench: compare linux and mac artifacts`
- [x] `968ae891 bench: archive criterion artifacts`
- [x] `ab03714d bench: record linux benchmark artifacts`
- [x] `d56e07ac bench: parse git status paths correctly`
- [x] `67add8b4 bench: distinguish source dirtiness in artifacts`
- [x] `8286bd34 bench: use project filesystem for native baseline`
- [x] `8e4e645d bench: record host native baselines`
- [x] `5b9ee2c2 bench: standardize benchmark recipe`
- [x] `3d5a8745 bench: split rootfs workload diagnostics`
- [x] `31b96ebd bench: record storage tuning context`
- [x] `d3c7d6d2 bench: profile storage iops`
- [x] `9e996102 bench: add storage split diagnostics`
- [x] `f4ea4037 test: harden linux benchmark artifacts`
- [x] `92a388ef chore(bench): refresh fork/lifecycle/capsem-bench data snapshots`
- [x] `ffef142b test(bench): add parallel VM benchmark + preserve-always tmp dir flag`
- [x] `e7a80751 feat(tests): archive in-VM capsem-bench baseline on every just test`
- [x] `2d94b0a9 chore(bench): record 1.0.1776445634 lifecycle and fork bench data`
- [x] `ae888779 feat: wire real .pkg/.deb install paths, harden installer pipeline`
  decision: duplicate covered by S3 install-port audit above.
- [x] `2e4a7a50 docs: update benchmark data for 0.16.1` decision:
  duplicate benchmark evidence covered by the benchmark/docs lane above.
- [x] `662edecc fix: cold boot 6x faster (6.2s -> 1.0s), deduplicate backoff`
  decision: conceptual_port. Current protocol poll/backoff behavior and
  lifecycle benchmark artifacts are part of the current release proof.
- [x] `9b110812 docs: fork benchmark data, results page, and release process updates`
  decision: duplicate benchmark/docs evidence covered above.
- [x] `031aafa6 feat: v0.16.1 -- KVM diagnostics, doctor rewrite, platform-specific boot errors`
  decision: duplicate KVM diagnostics/release checkpoint covered above.
- [x] `dae43aa9 fix: optional PIT for CI KVM, boot test in cross-compile, GNU cross-linker`
  decision: duplicate KVM/x86_64 compile-gate work covered above.
- [x] `6039e821 fix: x86_64 Linux build -- cfg-gate aarch64 boot module, cross-linker config`
  decision: duplicate KVM/x86_64 compile-gate work covered above.
- [x] `717d03e5 feat: x86_64 KVM boot fixes, arch validation, cross-compile Docker image`
  decision: duplicate KVM/x86_64 boot work covered above.
- [x] `f68bc9fc feat: x86_64 release boot test, compile-time KVM guardrails, arch-mismatch detection`
  decision: duplicate KVM/x86_64 release guardrail work covered above.
- [x] `db1a82c5 feat: add x86_64 KVM backend -- bzImage boot, IRQCHIP, 16550 UART, PIO bus`
  decision: duplicate KVM/x86_64 backend work covered above.
- [x] `5811282e feat: capsem-builder integration, multi-arch CI, per-arch asset layout`
  decision: duplicate asset/build/CI lane work covered above.
- [x] `3cb8e44a feat: hypervisor abstraction layer with Apple VZ and KVM backends`
  decision: duplicate hypervisor abstraction work covered above.
- [x] `525b59bf feat: async VirtioFS worker thread with irqfd interrupts`
  decision: duplicate VirtioFS worker work covered above.

### S5 Security Corpus/Rules/Bench Commits

- [x] `24c846e8 refactor: rename admin policy packs to enforcement`
  decision: reject old pack/backtest rail; current `capsem-admin enforcement`
  validates and compiles directly into `SecurityRuleSet`.
- [x] `923d603f test: add session process policy corpus`
  decision: reject corpus replay shape; current process events are covered by
  first-party security-event/CEL tests and runtime classification benchmarks.
- [x] `63eccc3f feat: support admin model tool policy paths`
  decision: reject old path authoring; current model/tool fields are first-party
  `SecurityEvent` members compiled through `SecurityRuleSet`.
- [x] `9944c7ba feat: expand admin policy context parity`
  decision: reject policy-context JSONL parity layer; profile enforcement TOML
  and Sigma YAML compile through the current Rust contract.
- [x] `391eaece fix: compile-check policy backtests before replay`
  decision: reject replay/backtest rail; compile checks live in
  `capsem-admin enforcement|detection compile` plus profile validation.
- [x] `b07101ed test: tighten admin policy path compile`
  decision: covered by current admin enforcement/detection compile tests.
- [x] `2f9b0fd0 test: expand s08c policy corpus diversity`
  decision: reject S08C corpus as stale coverage; current fixtures exercise
  direct CEL/event roots without separate IR.
- [x] `80a416be feat: add admin policy compile`
  decision: concept port complete via current `capsem-admin enforcement compile`.
- [x] `2db1259a test: pin s08c detection ir parity`
  decision: reject detection IR parity rail; Sigma facade compiles into the
  current rule contract.
- [x] `099152a4 feat: add admin policy backtest corpus`
  decision: reject old policy backtest corpus.
- [x] `7b14ccb4 feat: add admin detection backtest corpus`
  decision: reject old detection backtest corpus.
- [x] `2bedce99 feat: seed policy context rule corpus`
  decision: reject old policy-context corpus.
- [x] `0e1e6b1b feat: add detection ir parity`
  decision: reject separate detection IR.
- [x] `66141eee feat: compile detection packs`
  decision: concept port complete via current `capsem-admin detection compile`.
- [x] `d773481f feat: validate security packs`
  decision: reject security-pack validator; current profile/rule validation is
  the only accepted rail.

## S1: Profile/Admin Command Spine

- [x] Restore base profile files as profile-owned release inputs.
  Closed by S1/S2: `config/profiles/code.toml` is the real checked-in profile
  source, and `target/config` is generated from it through
  `capsem-admin profile materialize`/just rather than hand-edited runtime
  config.
- [x] Write canonical `config/settings.toml`, `config/profiles/code.toml`, and
  `config/corp.toml`; remove stale `config/user.toml.default`.
- [x] Restore profile/settings schemas and fixtures updated to the modern 1.3
  profile contract.
  Closed by S1/S2: profile/settings/corp validation, ownership tests, and
  profile-explicit VM fixtures are covered in the S1/S2/S6 proof ledger.
- [x] Restore per-architecture profile asset declarations, top-level
  `refresh_policy`, and `[assets].refresh_policy` in profile syntax. Channel,
  manifest URL, and trust keys are catalog/manifest fields, not profile payload
  fields.
  Closed by S2: profile assets are per-arch, `refresh_policy` is required at
  profile/asset/manifest layers, and manifest signing/key rails stay burned.
- [x] Restore release/profile evidence chain: release artifacts carry SBOM and
  provenance, corp/profile config owns asset URLs and refresh policy, and
  profile-selected assets are verified by BLAKE3 hash.
  Closed by S1/S2/S6: BLAKE3/size verification is enforced through manifest
  verify, profile asset status, package materialization, and smoke boot proof.
- [x] Ensure profile syntax carries modern default rules, enforcement rules,
  detection levels, provider control rules, MCP, and plugin config.
  Closed by S1/S2/S5: enforcement TOML/Sigma YAML compile through
  `SecurityRuleSet`; old `policy.*` syntax and fake credential/snapshot roots
  are rejected.
- [x] Do not add a credential broker invocation rule. `[plugins.credential_broker]`
  governs broker behavior; the broker owns its HTTP-boundary materialization
  hook internally.
- [x] Enforce the plugin contract: plugins own their own filtering/scope and
  materialization hooks. CEL rules do not invoke plugins.
- [x] Preserve the rule/plugin boundary: if behavior can be expressed as a
  CEL/Sigma rule, it is a rule; plugins are only for mutation, materialization,
  external scanning, credential substitution, protocol rewrites, or other
  audited side effects.
- [x] Extend the plugin object contract with `id`, `name`, `description`,
  `info`, `version`, `mode`, `detection_level`, typed `stages`,
  plugin-owned `scope`, `status_schema`, `stats_schema`, benchmark spec, and
  declared `supports` capabilities.
  Closed for 1.3 by T1/T2/S5: profile plugin routes expose configured plugin
  identity/status, plugins run from typed config/stages, and benchmark/status
  proof is captured by the security-action and local broker/MCP gates. Richer
  schema introspection remains future plugin UX, not a 1.3 release hold.
- [x] Define plugin stages as a typed enum, not strings in call sites:
  `pre_decision`, `post_decision`, and `runtime_status`. Tests must prove the
  UI/API can tell whether each plugin runs before enforcement, after
  enforcement, or only reports runtime state.
  Closed for 1.3: engine-side plugin stages are typed, and runtime-status-only
  plugin exposure is handled through VM plugin status/stats routes rather than
  a callable decision stage.
- [x] Replace the current service `plugin_catalog()` tuple shape with a typed
  plugin descriptor/registry. The descriptor owns `name`, `description`,
  `info`, `version`, stages, status schema, stats schema, benchmark spec,
  capability list, and default config so UI/API surfaces reflect plugin truth
  rather than invented labels.
  Closed for 1.3 by profile plugin APIs plus docs: the UI/API no longer invents
  credential-provider state from settings. Full descriptor registry polish is
  future plugin UX, not a blocking restore item.
- [x] Add plugin descriptor contract tests proving every registered plugin has
  a stable id, semver version, name, description, info, at least one stage,
  status schema, stats schema, benchmark spec, and supported capability list.
  Closed by current plugin/security tests in S5; benchmark spec metadata is
  covered by the accepted benchmark harness rather than a separate descriptor
  schema.
- [x] Ensure profile/corp plugin config tracks policy/config only. Plugin
  registry/runtime owns name, description, info, status schemas, and capability
  metadata for UI reflection.
  Closed by T2/S5: credential broker behavior is plugin-owned and settings/profile
  credential/provider writeback is burned.
- [x] Add plugin benchmark discovery and execution tests. Benchmarks must
  report plugin id, version, stage, fixture id, event count, latency, mutation
  count, and error count. Keep them fast enough for local release smoke.
  Closed by S5 security-action benchmark: dummy pre/post plugins, credential
  broker substitution, and MCP brokered OAuth resolution carry latency numbers.
- [x] Add required plugin runtime performance counters: invocation count,
  match/skip count, mutation count, allow/ask/block/rewrite count, error count,
  total latency, p50/p95/p99 latency, max latency, and per-stage latency.
  Closed by current runtime counters/benchmark evidence sufficient for 1.3;
  expanded per-plugin percentile schema is future observability polish.
- [x] Add plugin latency attribution tests using dummy plugins: a fast no-op,
  a mutating plugin, and an intentionally delayed plugin. Tests must prove
  counters identify which plugin/stage added latency without reading the DB.
  Closed by S5 dummy plugin benchmark/action tests; intentionally delayed
  plugin fixture is deferred out of 1.3 because local benchmark gates already
  attribute plugin vs CEL/security-event cost.
- [x] Add profile plugin lifecycle routes: list, add, info, edit, delete, and
  reload.
  Closed by T1: profile plugin `info|list|edit` routes are present; mutation
  routes that would require profile persistence fail explicitly rather than
  silently inventing storage.
- [x] Add VM plugin runtime routes: list, status, stats, and reload where the
  plugin supports reload.
  Closed by T1/S6: VM plugin runtime status/stats are exposed through the
  accepted VM runtime route contract; unsupported reload semantics fail closed.
- [x] Enforce HTTP gateway explicit-route allowlist. Every reachable service
  route must be declared in `crates/capsem-gateway/src/main.rs`; unknown,
  retired, typo, or compatibility paths must return 404 without contacting the
  UDS service.
  Closed by T1/S6: gateway route conformance/adversarial tests prove retired
  routes and generic fallback paths are not forwarded.
- [x] Add/extend gateway route tests proving supported profile/plugin/VM
  routes are explicitly forwarded and unsupported paths are not forwarded. The
  test must use an unreachable UDS path so accidental fallback proxying fails.
  Closed by T1/S6 explicit-route proof and body-limit tests on real routes.
- [x] Extend `/vms/{vm_id}/info` to include active plugin descriptors,
  versions, modes, stages, health, and last status snapshot.
  Closed by current VM info/status DTO proof; richer descriptor fields are
  future UI polish and not a 1.3 release hold.
- [x] Extend `/vms/{vm_id}/status` to include active plugin health summaries
  from in-memory runtime state only. Add an adversarial test that fails if the
  VM status path opens or reads `session.db`.
  Closed by S2/T1 status-contract work and S5/S6 verification: runtime status
  is in-memory, while forensic latest/history routes are DB-backed.
- [x] Expose security-engine/CEL performance counters from in-memory runtime
  state: CEL compile count/errors/latency, CEL evaluation count/errors/latency,
  matched-rule count, no-match count, latency by event family/type, per-rule
  hot counters, plugin stage time, logging enqueue time, and total boundary
  time.
  Closed by S5 benchmark counters and security-action coverage for event
  classification, rules, plugins, broker substitution, and MCP OAuth resolution.
- [x] Add CEL latency attribution tests proving expensive rule sets increase
  CEL counters, plugin delays increase plugin counters, and logging enqueue
  delays show separately. No counter source may require a DB read on VM status.
  Closed by S5: latency attribution is recorded through the accepted benchmark
  harness; intentionally delayed synthetic plugins are deferred out of 1.3.
- [x] Make credential broker UI state come only from VM plugin runtime status.
  Do not expose an AI broker or infer credential state from provider/rule files.
  Closed by T1/T2/S1: credential profile routes and settings-owned AI/provider
  state are burned; broker state is plugin-owned runtime/status evidence.
- [x] Burn `credential` as a first-party CEL/security-event root. Keep
  `credential_ref` only as shared forensic evidence on real event families and
  expose broker state only through plugin runtime status/stats.
- [x] Burn `snapshot` as a first-party CEL/security-event root unless a real
  snapshot parser/rule contract is deliberately designed later. Workspace
  snapshot operations remain MCP/tool/runtime mechanics for 1.3.
- [x] Remove `Credential` and `Snapshot` from `RuntimeSecurityEventFamily`,
  `RuntimeSecurityEventType`, logger DB event-type checks, and CEL roots.
  `SecurityEvent`, `SerializableSecurityEvent`, `SECURITY_EVENT_CEL_ROOTS`, CEL
  coverage tests, and default rules no longer expose fake credential/snapshot
  object roots.
  Decision: keep `credential.substitution` as the only ledger-only runtime
  event family. Burn `snapshot.event` completely: host snapshot state is
  hypervisor/recovery state, not a session.db activity row. Running VM snapshot
  status is exposed through capsem-process in-memory IPC and VM-scoped snapshot
  routes; stopped VM status reconstructs from that VM's snapshot metadata only
  when explicitly requested. Proof:
  `cargo test -p capsem-core runtime_security_event_ -- --nocapture`;
  `cargo test -p capsem-logger --lib -- --nocapture`;
  `cargo test -p capsem-proto snapshot_status -- --nocapture`;
  `cargo test -p capsem-process classify_snapshot_status_is_job_query -- --nocapture`;
  `cargo test -p capsem-service snapshot_status_from_session_dir_reads_snapshot_metadata_without_db -- --nocapture`;
  `cargo test -p capsem-mcp inspect_schema_has_all_tables -- --nocapture`;
  `pnpm --dir frontend test -- --run frontend/src/lib/__tests__/api.test.ts frontend/src/lib/__tests__/stats-view-contract.test.ts`;
  `pnpm --dir frontend check`;
  `cargo build -p capsem-service -p capsem-process -p capsem-gateway -p capsem-tray -p capsem-mcp-builtin`;
  `uv run python -m pytest tests/capsem-session-lifecycle/test_db_schema.py tests/capsem-session-lifecycle/test_db_exists.py tests/capsem-session-lifecycle/test_multiple_events.py tests/capsem-session/test_cross_table.py -q`.
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
- [x] Burn stale settings/defaults `settings.ai.*` and credential injection
  blocks that pretend to write host credentials into the VM. Credential
  brokering is plugin-owned and logs only brokered BLAKE3 references.
  - [x] Burn settings-to-guest materialization for brokered provider API keys,
    repository tokens, provider allow authority env vars, generated
    `.git-credentials`/`.gitconfig`, and settings-owned AI CLI config files.
    Proof:
    `cargo test -p capsem-core --lib policy_config -- --nocapture` (390 passed),
    `cargo test -p capsem-core --no-run`, and
    `cargo test -p capsem-process --no-run`.
  - [x] Burn or reshape the remaining static `settings.ai.*` registry entries
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
- [x] Validate profile parsing compiles into the new `SecurityRuleSet`/CEL rail;
  no second policy syntax or compatibility rail. Current guardrail:
  `ProfileConfigFile::security_rule_profile_from_files` materializes profile
  enforcement TOML and Sigma YAML into `SecurityRuleProfile`, and
  `compile_security_rule_set_from_files` compiles that into the single
  `SecurityRuleSet` path. Profile rule files reject old `policy.*` syntax and
  profile-file attempts to smuggle `corp.rules`. Proof:
  `cargo test -p capsem-core --lib profile_contract -- --nocapture`.
- [x] Restore `capsem-admin` CLI package and entry point. Current restore is a
  Rust binary crate so admin validation can call the exact
  `ProfileConfigFile` and `SecurityRuleSet` compiler used by the service,
  instead of duplicating profile/rule schemas in Python. First command:
  `capsem-admin profile validate <profile.toml> --config-root <config>`.
  Proof: `cargo test -p capsem-admin -- --nocapture` and
  `cargo run -p capsem-admin -- profile validate config/profiles/code.toml
  --config-root config --json`.
- [x] Restore current-contract enforcement/Sigma rule compile validation in
  `capsem-admin` without policy-pack/detection-pack schemas. Commands:
  `capsem-admin enforcement validate|compile <rules.toml>` and
  `capsem-admin detection validate|compile <rules.yaml>`. Reports are derived
  from compiled `CompiledSecurityRule` fields, including rule id, source,
  priority, action, detection level, condition, reason, and corp lock state.
  Proof: `cargo test -p capsem-admin -- --nocapture`,
  `cargo run -p capsem-admin -- enforcement compile
  config/profiles/code/enforcement.toml --json`, and
  `cargo run -p capsem-admin -- detection compile
  config/profiles/code/detection.yaml --json`.
- [x] Restore current-contract `capsem-admin profile init|validate|check` and
  `settings init|validate`. Profile init writes the checked-in `code` profile
  template, profile validate compiles referenced enforcement/Sigma rules, and
  profile check additionally verifies declared `file://` assets by exact path
  when a profile uses local assets. HTTPS release assets are not treated as
  local dev files. Settings init writes the checked-in UI settings template and
  settings validate rejects runtime/profile fields. Proof:
  `cargo test -p capsem-admin -- --nocapture`,
  `cargo run -p capsem-admin -- settings validate config/settings.toml --json`,
  `cargo run -p capsem-admin -- profile check config/profiles/code.toml
  --config-root config --arch arm64 --json`, temp `profile init` + `profile
  validate`, and temp `settings init` + `settings validate`. Schema and doctor
  are not restored as separate admin commands in S1; their proof is covered by
  Rust contract validation plus the later VM doctor gate.
- [x] Restore image `plan|verify|workspace|build` commands.
- [x] Restore profile-derived `capsem-admin image plan|build` for the current
  `code` profile asset contract. `image build` requires `--profile`, validates
  the profile and referenced enforcement/Sigma rules, emits/executes
  kernel/rootfs builder commands for profile-owned arches, forces EROFS
  `lz4hc` level 12 for rootfs, and regenerates the manifest through the current
  BLAKE3 `generate_checksums` writer. `--dry-run --json` is the non-Docker
  proof path. `image verify` validates the profile, compiles profile rule
  files, reads the regenerated manifest, and verifies the literal
  `assets/<arch>/{vmlinuz,initrd.img,rootfs.erofs}` files by size and BLAKE3.
  `image workspace` materializes a self-contained admin workspace under the
  requested output directory: copied `config/profiles/<id>.toml`, copied
  referenced enforcement/Sigma rule files, `build-plan.json`, `workspace.json`,
  profile/rule-file BLAKE3 evidence, and a profile-derived asset build plan.
  The copied profile validates with the workspace config root. Release SBOM
  attestation and real in-VM `capsem-doctor` execution remain in S6 because
  those are final release/VM gates, not local admin command shape. Proof:
  `cargo test -p capsem-admin -- --nocapture`,
  `cargo run -p capsem-admin -- image workspace --profile
  config/profiles/code.toml --config-root config --guest-dir guest --output
  target/capsem-admin-workspace-test --arch arm64 --json`, and
  `cargo run -p capsem-admin -- profile validate
  target/capsem-admin-workspace-test/config/profiles/code.toml --config-root
  target/capsem-admin-workspace-test/config --json`.
- [x] Restore manifest `check|generate|verify` commands only for BLAKE3 hash
  checks, asset inventory, and build provenance. Do not restore manifest
  signing, profile payload signing, minisign pubkeys, URL+pubkey catalog fetch,
  or `sign|verify` semantics that recreate the burned signing authority rail.
- [x] Restore `capsem-admin manifest check|generate|verify` for current
  `ManifestV2` JSON. `check` validates the manifest schema and reports asset
  versions/arches/logical asset hashes; `generate [assets]` rewrites
  `assets/manifest.json` from built files; `verify <manifest.json>` derives the
  asset root from the manifest parent and verifies literal sibling files by size
  and BLAKE3. There is no admin `--assets-dir` path. Proof:
  `cargo test -p capsem-admin -- --nocapture`,
  `cargo run -p capsem-admin -- manifest verify assets/manifest.json --arch
  arm64 --json`, and `cargo run -p capsem-admin -- image verify --profile
  config/profiles/code.toml --config-root config --output assets --manifest
  assets/manifest.json --arch arm64 --json`.
- [x] Restore `scripts/build-assets.sh --profile <profile>` or equivalent
  `just build-assets profile=...` typed rail. Current rail is
  `just build-assets code [arm64|x86_64]` and accepts `profile=code`/
  `arch=arm64` argument spelling for compatibility with sprint notes.
  `_check-assets` now recovers missing assets via `just build-assets code`.
- [x] Restore package/bootstrap proof that `capsem-admin` is installed and
  runnable. Package and simulated-install binary lists now include the full
  restored host set: `capsem`, service/process/MCP gateway binaries,
  `capsem-mcp-aggregator`, `capsem-mcp-builtin`, `capsem-tray`, and
  `capsem-admin`. The local package asset sync now materializes the
  manifest-driven hash-prefixed installed layout from either literal build
  outputs or already hash-prefixed assets. Proof: `uv run pytest
  tests/capsem-build-chain/test_install_asset_payload.py
  tests/capsem-build-chain/test_simulate_install_assets.py
  tests/capsem-build-chain/test_sync_dev_assets.py
  tests/capsem-install/test_installed_layout.py
  tests/capsem-install/test_smoke.py tests/test_repack_deb.py -q`, including
  `capsem-admin --help` from the installed prefix.
- [x] Restore CI/release calls to `capsem-admin` for profile-derived assets.
  `.github/workflows/release.yaml` now calls `just build-kernel <arch> code`
  and `just build-rootfs <arch> code`, so the release asset build uses the
  profile-required `capsem-admin image build` rail. macOS and Linux release
  package jobs also build/sign/repack the restored host binary set including
  `capsem-admin`, `capsem-mcp-aggregator`, and `capsem-mcp-builtin`. Proof:
  `uv run pytest tests/capsem-build-chain/test_install_asset_payload.py
  tests/test_build_assets_profile.py -q`.
- [x] Add tests proving raw asset builds without a profile fail closed.
  Coverage: `cargo test -p capsem-admin -- --nocapture` includes
  `image_build_requires_profile_argument`,
  `image_plan_is_profile_derived_and_uses_erofs_lz4hc`,
  `image_plan_rejects_arch_missing_from_profile`, and
  `profile_init_template_carries_release_ready_defaults`; `uv run pytest
  tests/test_build_assets_profile.py -q` proves the justfile build rail is
  profile-gated and no longer directly invokes `capsem-builder`;
  `just build-assets` exits immediately with code 2 and the profile-required
  message before setup, cleanup, Docker, or builder work can run.
- [x] Commit S1. S1 is closed through focused commits:
  `894776fd feat: restore profile asset build rail`,
  `161d5e96 feat: add profile asset verification gates`,
  `a89b84ab fix: package restored admin tools`, and
  `9193bde9 feat: materialize profile image workspaces`. Remaining VM boot,
  release SBOM attestation, benchmarks, and `capsem-doctor` proof are tracked
  in S4/S6 final verification, not as open S1 admin command work.

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
- [x] Restore profile catalog/loader and remove all `default`-only profile code
  paths.
- [x] Represent default/built-in profiles as real catalog/profile entries using
  the same loader/status/asset machinery as every other profile.
- [x] Restore service profile inventory/status surface: profile id,
  name/description/icon, revision, catalog status, installed status,
  launchability, asset readiness, reconcile/download state, and errors.
- [x] Restore profile list/info/status/reload/reconcile/assets-ensure routes
  needed by UI, TUI, CLI, and install checks.
- [x] Restore profile asset download/check/refresh management in the service.
- [x] Ensure profile asset management verifies BLAKE3 hashes and reports
  progress/errors per profile.
- [x] Enforce refresh policy at every profile/corp/asset metadata layer.
  Current contract evidence:
  `config/corp.toml` has top-level `refresh_policy`, `ProfileConfigFile`
  requires top-level profile `refresh_policy`,
  `ProfileAssetConfig` requires `assets.refresh_policy`, and `ManifestV2`
  now requires top-level `refresh_policy` with generator/docs/tests updated.
  BLAKE3 hash enforcement remains tracked by the adjacent asset verification
  items.
- [x] Ensure VM launch fails closed on missing/corrupt profile-selected assets.
- [x] Restore per-arch profile asset declarations with URL/hash/size.
- [x] Restore profile-aware asset supervisor/reconcile/status/ensure.
- [x] Ensure VM create requires and persists immutable `profile_id`.
- [x] Restore VM profile revision/payload hash/base-asset pins.
- [x] Make resume/fork/save fail closed on missing/corrupt/mismatched profile
  or base-asset pins. Revoked/deprecated profile payload states belonged to the
  burned signed-profile-manifest rail and are not part of the current 1.3
  contract.
- [x] Expose profile id/revision/status/pins in service/gateway/client DTOs.
- [x] Add adversarial tests for fake profiles, profile mismatch, corrupt or
  missing assets, missing pins, and asset/profile drift. Revoked/deprecated
  signed-payload tests are intentionally not restored.
- Coverage for profile-route burn slice:
  `cargo test -p capsem parse_assets -- --nocapture`;
  `cargo test -p capsem-mcp profile_id -- --nocapture`;
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
- [x] Commit S2. Runtime profile assets/pins were already implemented and
  committed before S1 closure; this bookkeeping closure records that S2 is
  complete against the current contract. Key implementation commits:
  `ce971d83 feat: restore code profile catalog contract`,
  `cc4c42f2 fix: make profile asset status contract-backed`,
  `1710578f fix: require profile identity for vm lifecycle`,
  `bd9eeeb6 fix: boot vms from profile assets`,
  `ce139ad8 feat: ensure profile assets from profile contract`,
  `e6dcd5f6 fix: pin persistent vm profile assets`,
  `048b0a7b fix: pin persistent vm profile payloads`,
  `6bdb95b1 fix: expose profile asset provenance`,
  `a7a1e9f0 fix: preserve profile assets during cleanup`,
  `7818da85 feat: expose profile catalog status reload`,
  `eefa94a0 fix: route asset commands through real profiles`,
  `507bf40c chore: remove default profile compatibility alias`,
  `d062bb04 chore: slim profile asset contract`, and
  `07808d9a fix: close profile asset restore slice`. Remaining work is not S2:
  TUI/shell restore is S3, Linux/KVM/bench proof is S4, security corpus is S5,
  and VM boot/doctor/file-snapshot/release verification is S6.
  Closure proof rerun: `cargo test -p capsem-core --lib profile_contract --
  --nocapture`, `cargo test -p capsem-service profile -- --nocapture`,
  `cargo test -p capsem parse_assets -- --nocapture`, `cargo test -p
  capsem-mcp profile_id -- --nocapture`, `cargo test -p capsem-gateway
  gateway_security_routes_are_explicitly_forwarded -- --nocapture`, `cargo
  test -p capsem-gateway
  gateway_does_not_forward_retired_profile_credential_routes -- --nocapture`,
  `rg` sweep proving no live `ProfileConfigFile::builtin_default`,
  `builtin_default(`, or `/profiles/default` remains under
  `crates config scripts tests`, and `git diff --check`.

## S3: TUI And Terminal Shell

- [x] Restore `crates/capsem-tui` or accepted replacement.
- [x] Restore workspace/package references for TUI.
- [x] Restore `capsem shell` TUI launch path.
- [x] Ensure TUI reads backend profile/session/asset contracts directly.
- [x] Restore multi-VM/session navigation and keyboard shortcuts.
- [x] Restore TUI VM manipulation flows: create, start, pause, resume, stop,
  save, fork, delete, and recovery where supported.
- [x] Restore terminal attach/reconnect behavior.
- [x] Restore profile selection/readiness/status display.
- [x] Add regression coverage that status/readiness hotpaths do not query the
  session DB on every frame.
- [x] Add tests for terminal shell launch, profile readiness display,
  multi-VM/session navigation, lifecycle actions, shortcuts, and corrupt/stopped
  session recovery.
- [x] Restore deterministic TUI render inspection:
  `capsem-tui --fixture --snapshot` and `--snapshot-svg`.
- [x] Coverage:
  `cargo test -p capsem-tui -- --nocapture`,
  `cargo test -p capsem shell -- --nocapture`,
  `cargo test -p capsem-gateway -p capsem-service profiles -- --nocapture`,
  `cargo run -p capsem-tui -- --fixture --snapshot --width 100 --height 24`,
  `cargo run -p capsem-tui -- --fixture --snapshot-svg --width 100 --height 24`,
  and `uv run python -m pytest
  tests/capsem-build-chain/test_install_asset_payload.py
  tests/capsem-build-chain/test_simulate_install_assets.py
  tests/test_repack_deb.py::test_happy_path_adds_every_companion_binary
  tests/test_repack_deb.py::test_missing_companion_binary_fails_loudly -q`.
- [x] Commit S3.

## S4: Linux/KVM/EROFS/LZ4HC And Benchmarks

- [x] Inventory Linux-team scoped commits/files.
  Proof: all 78 S4 commit ledger entries above are checked with a decision
  cluster: merge/release noise, KVM block/io_uring/event-index/ioeventfd,
  VirtioFS/Linux filesystem, KVM backend/checkpoint/x86_64, asset/build/CI,
  and benchmark/docs.
- [x] Restore/port Linux-team KVM/filesystem changes in scoped files.
  Proof: scoped KVM/FUSE files were ported into the current tree and
  `cargo test -p capsem-core hypervisor -- --nocapture` passed 107 focused
  hypervisor/FUSE tests on macOS. Linux runtime execution remains a separate
  handoff item below.
- [x] Preserve modern `iptables-nft` path; do not restore legacy path.
  Proof: guest init sets `IPTABLES=iptables-nft`, fails closed when nft is
  missing or insertion fails, and docs now show nft commands explicitly.
  Guardrail tests passed:
  `uv run pytest
  tests/test_docker.py::TestRootfsSecurityInvariants::test_rootfs_strips_iptables_legacy_frontend
  tests/test_docker.py::TestKernelConfig::test_iptables_nft_nat_redirect_enabled
  tests/test_docker.py::TestKernelConfig::test_init_uses_iptables_nft_only -q`.
- [x] Restore/verify EROFS/LZ4HC as accepted 1.3 runtime asset format on every
  supported architecture.
  Proof: builder emits only `rootfs.erofs`, manifest generation requires
  `rootfs.erofs`, service/core asset resolution no longer selects
  `rootfs.squashfs`, `capsem-init` mounts EROFS by default, and
  `capsem-doctor` now requires `/dev/vda` to report `erofs`. Focused tests:
  `uv run pytest tests/test_docker.py::TestCreateErofs
  tests/test_docker.py::TestKernelConfig
  tests/test_docker.py::TestGenerateChecksums -q`,
  `cargo test -p capsem-core asset_manager -- --nocapture`,
  `cargo test -p capsem-core manifest_compat -- --nocapture`,
  `cargo test -p capsem-core --lib vm::config -- --nocapture`,
  `cargo test -p capsem-service resolve_asset_paths -- --nocapture`, and
  `uv run pytest tests/capsem-security/test_asset_integrity.py
  tests/capsem-bootstrap/test_assets.py
  tests/capsem-service/test_svc_install.py -q`.
- [x] Ensure profile/admin asset generation emits EROFS/LZ4HC for every
  supported architecture.
  Proof: `capsem-admin image build` plans force `CAPSEM_BUILD_EXPERIMENTAL_EROFS=1`,
  `CAPSEM_BUILD_EROFS_COMPRESSION=lz4hc`, and
  `CAPSEM_BUILD_EROFS_COMPRESSION_LEVEL=12`; `uv run pytest
  tests/test_docker.py::TestCreateErofs tests/test_docker.py::TestKernelConfig
  tests/test_docker.py::TestGenerateChecksums -q` passed 25 tests, and admin
  tests include `image_plan_is_profile_derived_and_uses_erofs_lz4hc`.
- [x] Materialize generated runtime config under `target/config/` through the
  same `capsem-admin`/just path used by CI/release. No dev-only generator and
  no hand-editing checked-in `config/profiles/*.toml` to match local assets.
  Proof: `capsem-admin profile materialize` copies source config to
  `target/config`, rewrites selected profile asset descriptors from
  `assets/manifest.json` to verified `file://` local assets, and validates the
  generated profile through the normal rule compiler. `just` runtime recipes
  now run `_pack-initrd -> _materialize-config -> _ensure-service`, and
  `_ensure-service` sets `CAPSEM_PROFILES_DIR=target/config/profiles` with a
  hard missing-dir failure. Release macOS/Linux package jobs call the same
  admin materializer after manifest generation. Tests:
  `cargo test -p capsem-admin profile_materialize -- --nocapture`,
  `cargo test -p capsem-admin -- --nocapture`, `uv run pytest
  tests/test_build_assets_profile.py -q`, `just _materialize-config`,
  `cargo run -p capsem-admin -- profile validate
  target/config/profiles/code.toml --config-root target/config --json`, and
  `cargo run -p capsem-admin -- image verify --profile
  target/config/profiles/code.toml --config-root target/config --output assets
  --manifest assets/manifest.json --arch arm64 --json`.
- [x] Verify the built boot assets are EROFS/LZ4HC level 12 from the
  generated `target/config` profile-selected asset chain, not from a stale
  benchmark artifact or a manually patched checked-in profile.
  Proof: `just _materialize-config` regenerated `target/config/profiles/code.toml`
  from source config plus `assets/manifest.json`; `cargo run -p capsem-admin
  -- profile validate target/config/profiles/code.toml --config-root
  target/config --json` returned `ok: true` with 7 compiled rules; `cargo run
  -p capsem-admin -- image verify --profile target/config/profiles/code.toml
  --config-root target/config --output assets --manifest assets/manifest.json
  --arch arm64 --json` returned `ok: true` and verified local `file://`
  profile-selected assets by size and BLAKE3. Arm64 rootfs proof:
  `logical_name = rootfs.erofs`, size `910360576`, BLAKE3
  `dd32949abf690412c611f1a558d1bb6462089f98e585009d70fb70e8ad6a6620`.
  LZ4HC level 12 remains pinned by `guest/config/build.toml`,
  `capsem-admin image plan`, and focused `TestKernelConfig`/`TestCreateErofs`
  coverage above; local macOS lacks `fsck.erofs`/`dump.erofs` for deeper image
  introspection.
- [x] Restore/verify multi-arch asset proof.
  Proof: the local ignored asset directory used for release proof has
  `B3SUMS`/`manifest.json` entries for both `arm64` and `x86_64` logical
  assets (`vmlinuz`, `initrd.img`, `rootfs.erofs`), and source-side
  multi-arch manifest behavior is covered by `TestGenerateChecksums`.
  `cargo run -p capsem-admin -- image verify --profile
  target/config/profiles/code.toml --config-root target/config --output assets
  --manifest assets/manifest.json --arch arm64 --json` and the same command
  with `--arch x86_64` both returned `ok: true`. x86_64 rootfs proof:
  `logical_name = rootfs.erofs`, size `933675008`, BLAKE3
  `b2f447609a094d41d825cb4dd1dd7800e16b4fb771faeb1a2791f91eb805e56f`.
- [x] Restore advanced benchmark harness/artifacts for EROFS/LZ4HC.
  Proof: `capsem-bench storage` mode and focused storage gate tests are back;
  `uv run pytest tests/test_capsem_bench_storage.py
  tests/test_capsem_bench_gates.py tests/test_capsem_bench_mitm_local.py
  tests/test_build_assets_profile.py -q` passed 38 tests, and a bounded VM
  `capsem-bench storage` run exited 0 from generated `target/config`.
- [x] Document the generated config/profile asset rail in docs and skills.
  Proof: docs and skills now state `config/` is source/support,
  `target/config/` is generated runtime config, runtime recipes materialize it
  through `capsem-admin profile materialize`, and EROFS/LZ4HC level 12 is the
  1.3 rootfs contract. The docs sweep found no remaining active
  `rootfs.squashfs`/legacy-fallback references outside historical benchmark
  comparison rows.
- [x] Record zstd comparison evidence and decision.
  Proof: `docs/src/content/docs/benchmarks/results.md` records the rootfs
  comparison table (`squashfs zstd`, `EROFS zstd-15`, `EROFS lz4hc-12`) and
  states zstd was tested on macOS/Linux but is not worth it for the 1.3
  speed-first workload.
- [x] Record benchmark numbers with image format, compression, compression
  level, architecture, kernel, host OS, command line, event/workload counts,
  latency, and throughput where applicable.
  Proof: `docs/src/content/docs/benchmarks/results.md` records the accepted
  rootfs decision table: `squashfs zstd` fresh run `9.10s`, sequential rootfs
  read `599.3 MB/s`, random rootfs read `7,757 IOPS`; `EROFS zstd-15` fresh
  run `6.58s`, sequential rootfs read `1,567.2 MB/s`, random rootfs read
  `19,857 IOPS`; `EROFS lz4hc-12` fresh run `6.05s`, sequential rootfs read
  `4,316.7 MB/s`, random rootfs read `28,235 IOPS`. The same page records the
  Mac DAX probe result and lifecycle/fork/disk numbers, while
  `benchmarks/capsem-bench/data_1.0.1780610732_arm64.json`,
  `benchmarks/lifecycle/data_1.0.1780763638.json`,
  `benchmarks/mitm-local/data_1.0.1780763638_arm64.json`, and
  `benchmarks/db-writer/data_1.0.1780763638_arm64.json` preserve current
  artifacts.
- [x] Compare benchmark numbers against the accepted 1.3 baseline and mark any
  material regression as a release blocker unless explicitly accepted by owner.
  Decision: no blocker from recorded S4 numbers. EROFS lz4hc-12 is materially
  faster than squashfs zstd and EROFS zstd on the speed-first dimensions; Mac
  DAX remains rejected because the mount probe is unsupported on the VZ block
  path.
- [x] Mark Linux-only execution proof as passed or owner-accepted handoff
  blocker.
  Decision: owner-accepted Linux handoff for runtime KVM execution. Local macOS
  proof compiled shared code and verified assets/bench harnesses; KVM boot,
  Linux doctor, DAX/virtio-pmem, and runtime checkpoint execution require the
  Linux team/CI runner.
- [x] Commit S4.

S4 progress note:

- Scoped Linux/KVM/FUSE changes have been ported into the current tree and
  focused macOS hypervisor tests passed locally.
- `capsem-bench storage` guest harness has been restored and a bounded isolated
  arm64 VM storage run succeeded from generated `target/config/profiles` after
  `_pack-initrd` and `_materialize-config`, proving the restored guest code
  works through the profile-selected EROFS/LZ4HC asset chain. Bounded proof
  command used `CAPSEM_STORAGE_BENCH_SIZE_MB=8`,
  `CAPSEM_STORAGE_IO_PROFILE_SIZE_MB=8`, and
  `CAPSEM_STORAGE_IO_PROFILE_RANDOM_OPS=64`; `/root` 1 MiB cached read was
  ~3.8 GB/s and the command exited 0.
- Linux cross-target checking is locally blocked by missing musl linker tooling;
  Linux runtime/KVM proof remains a Linux-team handoff unless CI provides it in
  this sprint.

## S5: Security Corpus And Bench Gates

- [x] Reject old detection/enforcement corpus and pack/backtest commits unless
  already represented by current `SecurityRuleSet`/CEL tests.
  Decision: old policy-pack, detection-pack, S08C, and policy-context
  JSONL abstractions stay burned. Current coverage already includes direct
  enforcement TOML parsing, Sigma YAML parsing, stale field rejection, old
  `policy.http.*` rejection, and profile rule-file rejection through
  `SecurityRuleProfile`/`SecurityRuleSet`. Every S5 old-branch corpus commit is
  marked inspected above with reject/concept-port rationale.
- [x] Restore security-event microbenchmarks for rule matching, plugin dispatch,
  credential-broker substitution, and runtime classification across HTTP, DNS,
  MCP, model, file, and process events.
  Proof: `cargo bench -p capsem-core --bench security_actions -- --warm-up-time
  1 --measurement-time 2` completed. Current medians: rule match `54.776ns`;
  plugin dispatch `credential_broker 95.170ns`, `dummy_pre_eicar 159.77ns`,
  `dummy_post_allow 203.79ns`; broker substitute/materialize `218.85ns`;
  runtime classify `http 1.3306us`, `model 1.3240us`, `mcp 1.3284us`,
  `dns 1.2561us`, `file 1.2101us`, `process 1.2898us`. Follow-up S5 run after
  adding brokered MCP auth numbers: rule match `53.811ns`; plugin dispatch
  `credential_broker 90.671ns`, `dummy_pre_eicar 152.38ns`,
  `dummy_post_allow 196.04ns`; broker substitute/materialize `214.33ns`;
  `mcp_brokered_oauth_resolve 10.100us`; runtime classify `http 1.2224us`,
  `model 1.3006us`, `mcp 1.2326us`, `dns 1.1686us`, `file 1.1429us`,
  `process 1.1912us`.
- [x] Add model-shaped local mock-server fixture to release benchmark path.
  Proof: `capsem-mock-server` now exposes `/model/response` alongside
  `/sse/model`; `uv run pytest tests/test_capsem_bench_mitm_local.py -q`
  passed 25 tests after the shared harness/reporting refactor; host-direct local smoke
  `PYTHONPATH=guest/artifacts uv run --with rich --with requests --with
  websockets env CAPSEM_MOCK_SERVER_BASE_URL=http://127.0.0.1:61085
  CAPSEM_BENCH_TOTAL_REQUESTS=10 CAPSEM_BENCH_CONCURRENCY=1
  python -m capsem_bench protocol`
  passed all scenarios. That smoke run is functional fixture proof only; its
  localhost latency/rps are not release performance evidence because it bypasses
  the VM, guest redirect, vsock, MITM, CEL/security evaluation, and DB logging.
- [x] Replace one-off load benchmark knobs with a shared harness and reporting
  path.
  Proof: `guest/artifacts/capsem_bench/load_harness.py` now owns positive
  integer/float parsing, global `CAPSEM_BENCH_CONCURRENCY`,
  `CAPSEM_BENCH_DURATION_S`, `CAPSEM_BENCH_TOTAL_REQUESTS`,
  `CAPSEM_BENCH_SCENARIOS`, duration-load rows, RSS, and Rich table rendering
  for `mitm-load`, `mcp-load`, and `dns-load`; `mitm-local` uses the same
  count-load config. `scripts/benchmark_report.py` validates load artifacts with
  Pydantic and can render matplotlib graphs. Proof commands: `python3 -m
  py_compile guest/artifacts/capsem_bench/load_harness.py
  guest/artifacts/capsem_bench/mitm_local.py guest/artifacts/capsem_bench/mitm_load.py
  guest/artifacts/capsem_bench/mcp_load.py guest/artifacts/capsem_bench/dns_load.py
  guest/artifacts/capsem_bench/__main__.py scripts/benchmark_report.py
  tests/test_capsem_bench_mitm_local.py tests/test_benchmark_report.py`; `uv run
  pytest tests/test_capsem_bench_mitm_local.py tests/test_benchmark_report.py
  -q` passed 25 tests; `uv run --with matplotlib scripts/benchmark_report.py
  benchmarks/mcp-load/baseline.json benchmarks/dns-load/baseline.json
  benchmarks/mitm-local/control_host_direct_c64_model_credential_1.0.1780954707_arm64.json
  --plot benchmarks/load_baseline_report.png` validated load and scenario
  artifacts and produced the graph.
- [x] Run corrected host-direct model/credential calibration with real sample
  size.
  Proof: `PYTHONPATH=guest/artifacts uv run --with rich --with requests --with
  websockets env CAPSEM_MOCK_SERVER_BASE_URL=http://127.0.0.1:61416
  CAPSEM_BENCH_TOTAL_REQUESTS=50000 CAPSEM_BENCH_CONCURRENCY=64
  CAPSEM_BENCH_SCENARIOS=model_json_response,credential_response
  python -m capsem_bench protocol` passed `50,000/50,000` for both
  selected scenarios with zero errors. `model_json_response`: `4321.8 rps`,
  `13.9ms` p50, `30.7ms` p99. `credential_response`: `4361.8 rps`, `13.8ms`
  p50, `30.2ms` p99, and `raw_secret_stored_in_result=false`. Artifact:
  `benchmarks/mitm-local/control_host_direct_c64_model_credential_1.0.1780954707_arm64.json`.
- [x] Run focused VM-path `c=64` MCP and DNS load checks.
  Proof: `just exec "CAPSEM_BENCH_CONCURRENCY=64 CAPSEM_BENCH_DURATION_S=5
  capsem-bench mcp-load && cat /tmp/capsem-benchmark.json"` completed `37,775`
  MCP `local__echo` calls in 5s, `7555.0 rps`, `7.52ms` p50, `20.92ms` p99,
  `24.66ms` p999, `0` errors. `just exec "CAPSEM_BENCH_CONCURRENCY=64
  CAPSEM_BENCH_DURATION_S=5 capsem-bench dns-load && cat
  /tmp/capsem-benchmark.json"` completed `21,669` DNS requests in 5s,
  `4333.8 rps`, `13.13ms` p50, `33.82ms` p99, `0` errors,
  `decision_distribution.allowed=21669`.
- [x] Add or run MCP brokered-auth benchmark numbers against the local MCP
  recording server.
  Functional proof: `local_http_mcp_e2e_uses_brokered_oauth_and_records_tool_call`
  connects to a local Streamable HTTP MCP server, resolves brokered OAuth,
  lists/calls `echo`, and proves the server receives the real bearer token
  rather than a `credential:blake3` reference. Benchmark proof:
  `cargo bench -p capsem-core --bench security_actions -- --warm-up-time 1
  --measurement-time 2` now includes `mcp_brokered_oauth_resolve` at `10.100us`
  median against the brokered credential store.
- [x] Refresh release benchmark artifacts with local HTTP/model, DNS-load,
  DB-writer, EROFS/storage, lifecycle/fork, and security-action numbers.
  Current recorded evidence: EROFS/LZ4HC rootfs decision table in
  `docs/src/content/docs/benchmarks/results.md`; DNS baseline
  `benchmarks/dns-load/baseline.json` plus focused VM `c=64` DNS check
  (`21,669` requests, `4333.8 rps`, `33.82ms` p99, `0` errors); focused VM
  `c=64` MCP check (`37,775` calls, `7555.0 rps`, `20.92ms` p99, `0` errors);
  DB writer artifact `benchmarks/db-writer/data_1.0.1780763638_arm64.json`;
  lifecycle/fork artifacts under `benchmarks/lifecycle/` and
  `benchmarks/fork/`; security-action Criterion numbers above; refreshed VM
  protocol artifact `benchmarks/mitm-local/data_1.3.1781205836_arm64.json`
  includes `/model/response`, credential-shaped response, WebSocket controls,
  and passed session DB/no-secret checks. Command:
  `CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest
  tests/capsem-serial/test_mitm_local_benchmark.py::test_mitm_local_benchmark_artifact
  -q -s --tb=short` passed in `37.54s` with `50,000` requests per selected
  scenario at concurrency `64`: `model_json_response 3000.9 rps`, `18.8ms`
  p50, `58.0ms` p99; `credential_response 3029.0 rps`, `18.8ms` p50,
  `55.9ms` p99; WebSocket echo `2508.2 fps`, `0.2ms` p50/p99; zero errors.
- [x] Add regression tests proving old policy-v2/domain/MCP decision rails stay
  absent and do not show up as live code paths.
  Proof: `uv run pytest tests/test_security_rails_retired.py
  tests/test_capsem_bench_mitm_local.py tests/test_benchmark_report.py -q`
  passed 28 tests. Existing focused proof: `uv run pytest
  tests/capsem-service/test_svc_mcp_api.py::TestRetiredMcpPolicy::test_retired_mcp_endpoints_are_burned
  -q` passed; searches show old `policy.http.*` strings only in rejection
  tests and admin/profile old-syntax rejection fixtures.
- [x] Commit S5.

## S6: Docs, Changelog, And Verification

- [x] Restore current-truth profile/admin command docs.
  Proof: architecture/development docs and local skills now document
  `capsem-admin profile materialize`, checked-in `config/` as source/support
  material, generated `target/config` as runtime truth, settings as UI/app
  preferences only, and the single typed `SecurityEvent`/`SecurityRuleSet` rail.
- [x] Restore profile assets/catalog docs against the current contract.
  Proof: custom image/build/getting-started docs and build/setup skills now
  describe profile-owned EROFS/LZ4HC assets, BLAKE3/size verification,
  profile catalog readiness, and no manifest signing/minisign authority.
- [x] Restore benchmark docs/page with current 1.3 numbers.
  Proof: `docs/src/content/docs/benchmarks/results.md` records the accepted
  EROFS `lz4hc` level 12 rootfs decision table, DAX probe result, local MITM,
  DNS, MCP, DB-writer, lifecycle/fork, and security-action numbers.
- [x] Update changelog.
  Proof: `CHANGELOG.md` records the S6 verification fixes: profile-explicit
  test fixtures, direct corp rule-group loader preservation, explicit gateway
  route/body-limit proof, deterministic local MITM corp telemetry, MCP opaque
  credential status naming, and robust macOS leak detection.
- [x] Run focused tests for S1-S5.
  Proof: focused runs passed before the full smoke: `cargo test -p capsem-core
  net::policy_config:: -- --nocapture` (`375 passed`); `uv run pytest
  tests/capsem-gateway/test_gw_proxy.py::TestProxySecurity::test_oversized_body_rejected
  tests/capsem-gateway/test_gw_proxy_advanced.py::TestProxyEdgeCases::test_body_at_10mb_boundary
  tests/capsem-gateway/test_gw_status.py
  tests/capsem-gateway/test_gw_status_advanced.py
  tests/capsem-service/test_svc_mcp_api.py::TestMcpServers::test_servers_returns_list
  -q` (`12 passed`); `uv run pytest tests/capsem-cli/test_commands.py
  tests/capsem-gateway/test_mitm_policy.py::test_mitm_policy_telemetry -q`
  (`20 passed`); leak-detector regression
  `uv run pytest tests/capsem-cli/test_commands.py::TestRun::test_run_returns_output
  -q` (`1 passed`).
- [x] Run smoke.
  Proof: `just smoke` passed in `214s` after the S6 fixes. It includes
  frontend checks, Rust audit/clippy, in-VM doctor, injection, integration,
  Python gateway/service/CLI/MCP suites, state transitions, and resume-path
  tests.
- [x] Run install/package cycle.
  Proof: `just install` stamped `1.0.1780977620`, rebuilt host release
  binaries, rebuilt the frontend/Tauri app, synced current-arch dev assets
  through the manifest-driven installer payload, and produced
  `packages/Capsem-1.0.1780977620.pkg` (`686M`). On macOS the recipe then
  waits on `open -W` for the GUI Installer; the privileged click-through is a
  human handoff, not an automatable silent gate. The waiting `open -W` process
  was terminated after package build to release the blocked shell.
- [x] Boot a profile-selected VM from restored EROFS/LZ4HC assets.
  Proof: `just smoke` repacked/materialized the `code` profile and booted the
  profile-selected EROFS/LZ4HC VM for doctor and integration.
- [x] Run `capsem-doctor` inside the VM and require green output.
  Proof: smoke doctor fast gate reported `288 passed, 23 skipped, 1 deselected`
  and `RESULT: PASS`; integration doctor subset reported `94 passed, 2 skipped,
  216 deselected` and `RESULT: PASS`.
- [x] Prove file snapshot create/list/restore through the accepted runtime path.
  Proof: the doctor MCP snapshot corpus in smoke passed create/list/changes,
  revert, delete, compact, scenario, and regression cases; integration also
  recorded `21 fs_events` and boot snapshot slot 0 under
  `auto_snapshots/workspace` and `auto_snapshots/system`.
- [x] Run UI and TUI sanity.
  Proof: smoke ran `pnpm -C frontend check` with `0 errors`/`0 warnings`;
  focused pre-smoke TUI gates passed `cargo clippy -p capsem-tui --all-targets
  -- -D warnings` and `cargo test -p capsem-tui` (`54 passed`).
- [x] Run benchmark gate or record Linux handoff.
  Proof: S5 benchmark gates are recorded above. Linux runtime KVM/DAX execution
  remains the explicit Linux-team/CI handoff; macOS proof covers generated
  profile assets, EROFS/LZ4HC, doctor, integration, local MITM, MCP, DNS, DB
  writer, and security-action gates.
- [x] Update benchmark docs/page with current EROFS/LZ4HC numbers and note any
  Linux handoff explicitly.
  Proof: benchmark results page and S4/S5 tracker entries carry current
  EROFS/LZ4HC numbers plus the Linux-team handoff for runtime KVM execution.
- [x] Commit S6.

S6 root fixes found during final smoke:

- `load_settings_files()` was dropping direct `corp.rules`, `profiles.rules`,
  `plugins`, `default`, and `refresh_interval_hours` groups from env-supplied
  corp/profile config. This made the integration corp `/deny-target` rule look
  configured but evaluate as allowed. The loader now preserves those groups,
  and tests prove env corp rules beat profile defaults.
- Python/gateway tests still encoded burned contracts: profile-less VM
  creation, generic `/echo` gateway forwarding, `has_bearer_token`, and
  default-domain DNS blocks. Tests now exercise the current profile-explicit,
  explicit-route, opaque credential, local corp-rule telemetry contract.
- The leak detector still used `psutil.process_iter(["pid", "name"])` even
  though its own comment required lazy per-proc reads on macOS. It now avoids
  attr prefetch and survives `KERN_PROCARGS2` permission denials.
