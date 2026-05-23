# S18 - Full Verification And Release Gate

## Goal

Prove the Profile V2 bedrock release is releaseable.

This gate is the Iron Bank. It does not certify a prototype, a partially usable
backend, or a future promise. It certifies that the engine split, signed profile
contract, runtime enforcement/detection, CLI, UI, docs, install path, VM path,
logs/status/debug, and benchmark claims stand together.

## Tasks

- Run backend tests for settings, profiles, assembly, APIs, CLI, enforcement.
- Run frontend tests for settings, profiles, runtime enforcement/detection
  overlays, VM create, logs/status/debug links, and security capabilities.
- Prove S08b engine split: Network/File/Process engines feed the Security
  Engine and Resolved Event Emitter through typed contracts; no shipped event
  family bypasses the canonical resolved-event journal or reintroduces the old
  policy runtime.
- Prove UDS/HTTP/CLI/UI contract alignment: profile, enforcement, detection,
  status/log/debug, and VM-create surfaces use the same route names, typed
  payloads, enum values, error semantics, and evidence fields.
- Run E2E profile create/fork/delete/select/launch.
- Run manifest/profile-catalog install/update/remove/revoke tests.
- Run profile-backed VM create with missing assets to prove first-use download,
  signature/hash verification, VM pinning, and successful boot.
- Run resume-after-profile-update tests to prove existing VMs keep their pinned
  profile revision and asset hashes.
- Prove MCP, skills, AI providers, credential brokerage, PII, and canonical
  rules enforce through VM-effective settings.
- If credential brokerage is not shipped in the bedrock cut, prove no shipped
  profile or docs page advertises credential release as available; S10 owns the
  later implementation.
- If quotas/rate limits are not shipped in the bedrock cut, prove no shipped
  profile or docs page advertises budget enforcement as available; S22 owns the
  later implementation.
- Prove fresh install still works after v1 removal.
- Prove asset cleanup preserves files referenced by installed active/deprecated
  profile revisions and existing VM pins, and removes unreferenced revoked
  profile assets.
- Prove rollback and revocation behavior:
  stale signed manifest cannot downgrade an installed active profile; revoked
  profile revisions cannot create new VMs; existing revoked VM behavior matches
  the S07a contract and is visible in status/debug.
- Prove profile status enum consistency:
  `ProfileRevisionStatus` is the only representation for profile revision
  lifecycle state across manifest parsing, Rust models, Pydantic admin models,
  UDS/HTTP payloads, CLI output, UI models, status/debug reports, and docs. All
  three values (`active`, `deprecated`, `revoked`) have golden tests and
  user-facing semantics. `removed` is not accepted as a status; absent revisions
  are modeled as absent/unknown.
- Prove first-use download safety under concurrency:
  two simultaneous VM creates for the same profile revision do not corrupt
  partial files, duplicate network work unnecessarily, or race cleanup.
- Prove package/tool contract at runtime:
  a capsem-doctor or equivalent in-guest probe reads declared versions from the
  selected profile revision and verifies the booted VM actually contains them.
- Prove forward-only VM identity:
  persistent VM registry entries without a profile pin or pinned asset identity
  fail closed before process spawn; they never silently bind to the current
  catalog default.
- Prove `capsem-admin` packaging:
  bootstrap and release packages install the admin CLI; packaged
  `capsem-admin profile validate`, `manifest check --fast`, and `image verify`
  run successfully from the installed layout.
- Prove bootstrap path:
  developer bootstrap installs the local editable admin tooling with uv
  (`uv sync` / `uv pip install -e .` as finalized by S07b), not by consuming a
  release package.
- Prove profile-derived images:
  release image builds derive package/tool/image settings from selected
  profiles, not hand-edited `guest/config`; tests fail if builder inputs bypass
  the profile source of truth.
- Prove all-arch default:
  omitted `--arch` on `capsem-admin image build`, `image verify`, and manifest
  checks means `all` and covers every supported release arch. Single-arch mode is
  tested only as a narrowing override.
- Prove manifest admin checks:
  `capsem-admin manifest check --fast` validates remote profile/asset URLs with
  HTTP `HEAD`, while `--download` downloads and verifies all referenced bytes.
- Prove profile schema closure:
  `capsem-admin profile validate` rejects unknown fields/tables, wrong
  `capsem.profile.v2` schema id/version, manifest/payload id or revision
  mismatches, malformed package versions, unsupported arch declarations, and
  incomplete per-arch asset records. Rust and Python validators must pass the
  same JSON Schema Draft 2020-12 valid/invalid fixtures.
- Prove admin type safety:
  Python admin workflows use Pydantic v2 models for profile, manifest, asset,
  package/tool, build-plan, doctor, and report shapes. Tests fail if workflows
  bypass models with untyped nested dict manipulation, `json.loads`, or
  `json.dumps`. JSON input tests must go through `model_validate_json()` or
  `TypeAdapter.validate_json()`, and JSON output tests must go through
  `model_dump_json()`.
- Prove release docs truth:
  S19 pages document the bedrock contract and identify S10 credential brokerage,
  S22 quotas/rate limits, S13 remote plugins, S16a workbench polish, S19a
  marketing refresh, S20/S21 product expansions, and S19b reporting setup as
  later work unless they actually landed before this gate.
- Run the S19 documentation review checklist below and paste the exact command
  outputs, grep summaries, and any accepted historical/developer-only matches
  into this gate before release.

## S19 Documentation Review Checklist

Release docs are part of the product contract. The gate must prove they match
the shipped bedrock, not the historical implementation.

- Build the docs site: `pnpm --dir docs run build`.
- Search for stale runtime authority language:
  `rg -n 'guest/config|defaults\.json|config/defaults|\[mcp\]|NetworkPolicy|domain_policy|policy_config|security preset' docs/src/content/docs -S`.
  Every match must be one of:
  historical release notes, explicit developer-only built-in-profile caveat, or
  a statement saying the old path is not runtime/operator authority.
- Confirm the Profile Status enum docs use only `active`, `deprecated`, and
  `revoked`; `removed` must only appear in text explaining that it is not a
  valid status.
- Confirm docs describe signed manifests as profile catalogs with profile id,
  revision, status, payload identity, asset identity, and VM pins.
- Confirm docs describe Service Settings V2 separately from Profile V2 and do
  not claim generated UI descriptor/default artifacts are runtime authority.
- Confirm `capsem-admin` docs cover enterprise PyPI install, developer editable
  install, Pydantic-only JSON I/O, profile validate/schema, image plan/build/
  verify, manifest generate/check/sign, `--fast` HEAD checks, full download
  checks, omitted `--arch` defaulting to all supported arches, and JSON reports.
- Confirm detection and enforcement are documented as separate surfaces:
  detection can validate/backtest/hunt and emit findings; enforcement can
  allow, ask, block, or rewrite synchronously.
- Confirm authored enforcement examples use canonical DSL roots such as
  `http.request.host`, `http.request.url`, `http.request.path`,
  `http.request.header("authorization").exists()`, and
  `http.request.body.text`, not internal `event.*`.
- Confirm docs name S10 credential brokerage, S22 quotas/rate limits, S13
  remote plugins, S16a/S17 richer UI, S19a marketing, S20/S21 product
  expansions, and S19b reporting setup as future lanes unless they have fully
  passed this gate.
- Confirm release pages link operators to profile, catalog, corporate
  deployment, corporate security, VM health, telemetry extension,
  add-enforcement, and add-detection pages without requiring raw SQL or curl to
  understand the shipped path.

## Coverage Ledger

- Unit/contract: complete for profile catalog schema, `capsem.profile.v2`
  JSON Schema Draft 2020-12 closure, shared Rust/Python schema fixture parity,
  Pydantic v2 model coverage for every admin data shape, Pydantic-only JSON I/O
  coverage, signatures/hashes, `ProfileRevisionStatus` enum parity, package/
  tool contracts, per-arch assets, rollback protection, resolver inheritance,
  VM pin metadata, and API/CLI/UI shapes.
- Functional: complete for manifest update, profile install/update/remove/
  revoke, first-use asset download, VM create/resume/fork/delete, cleanup
  retention, explicit profile selection through UDS/HTTP/CLI/UI,
  enforcement/detection runtime registry and backtest/hunt surfaces, and
  `capsem-admin` profile/image/manifest workflows.
- Adversarial: complete for malformed manifests/profiles, bad signatures,
  truncated hashes, unauthorized profile signing key, unsupported arch,
  incompatible binary, revoked/deprecated revisions, absent/unknown revisions,
  partial downloads, cleanup races, path traversal, bad URL schemes, and stale
  catalogs.
- E2E/VM: complete for profile-backed VM boot, package/tool contract proof,
  enforcement through VM-effective settings, resume after catalog update, and
  cleanup safety with at least one persistent VM pin. At least one release-gate
  image is built or fixture-built from profile-derived inputs and verified in a
  booted VM.
- Telemetry: complete for debug/status/reporting of chain-of-trust state,
  profile revision, package contract, asset readiness, verification failures,
  VM pins, drift, revocation, and operator overrides.
- Performance: complete or explicitly waived with rationale; list/status do not
  hit network or perform expensive hash verification, and concurrent first-use
  downloads are bounded and deduplicated.

## Progress Journal

- 2026-05-23: Started S18 with the S19 documentation review replay.
  Verification commands:
  - `pnpm --dir docs run build` passed; Starlight generated 69 pages.
  - `rg -n 'guest/config|defaults\.json|config/defaults|\[mcp\]|NetworkPolicy|domain_policy|policy_config|security preset|allow list|domain policy' docs/src/content/docs -S` produced only accepted matches: historical release notes, service-settings fixture filenames, explicit developer-only built-in-profile caveats, and text explaining old paths are not runtime/operator authority.
  - `rg -n "^- \[ \]|^- \[~\]" sprints/policy-settings-profiles/S19-documentation-and-site.md` returned no open S19 checklist items.
  - `rg -n "removed" docs/src/content/docs/configuration docs/src/content/docs/architecture docs/src/content/docs/security docs/src/content/docs/getting-started -S` showed `removed` only in allowed prose: absent assets can be removed, file activity uses deleted/removed wording, old runtime was removed, and docs explicitly state `removed` is not a valid profile status.
  - `rg -n "active|deprecated|revoked|ProfileRevisionStatus" docs/src/content/docs/configuration docs/src/content/docs/architecture docs/src/content/docs/security -S` confirmed the public profile-status vocabulary is `active`, `deprecated`, and `revoked`.
  Fix applied during replay: updated the session-telemetry HTTP header-strip example from `policy.http.strip_credentials` to `security.rules.http.strip_credentials` so examples use the shipped Security Engine rule namespace.
- 2026-05-23: Continued S18 with the first contract/runtime replay slice.
  Verification commands:
  - `uv run python -m pytest tests/test_service_settings.py tests/test_profiles.py tests/test_admin_cli.py tests/test_security_packs.py -q` passed with 87 tests.
  - `cargo test -p capsem-security-engine` passed with 41 tests.
  - `cargo test -p capsem-core service_settings --lib`, `cargo test -p capsem-core profile_manifest --lib`, and `cargo test -p capsem-core --test profile_schema` passed after a first combined cargo invocation was rejected as invalid syntax.
  - `cargo test -p capsem-service handle_profile_catalog --bin capsem-service`, `handle_reconcile_profile_catalog`, and `vm_profile_pin` passed with 2, 3, and 5 focused service tests.
  - `pnpm --dir frontend exec vitest run src/lib/__tests__/session-runtime-truth.test.ts src/lib/__tests__/runtime-security-rules-section.test.ts src/lib/__tests__/profile-catalog-section.test.ts src/lib/__tests__/security-engine-health-section.test.ts src/lib/__tests__/api.test.ts` passed with 85 frontend tests.
  - `cargo test -p capsem-service enforcement --bin capsem-service`, `detection`, and `runtime_security_rule` passed with 8, 8, and 3 service runtime tests; the enforcement slice required unsandboxed Unix-socket permissions after the sandbox reported `Operation not permitted`.
  - `uv run python -m pytest tests/capsem-gateway/test_gw_proxy_advanced.py -q` passed with 25 gateway proxy tests.
  - `uv run python -m pytest tests/test_admin_docs.py tests/test_security_packs.py tests/test_admin_cli.py -q` passed with 62 admin/docs/security-pack tests after the release gate fixed the remaining public naming drift from `policy` to `enforcement`.
  - `pnpm --dir docs run build` passed again with 69 pages.
  - `uv run capsem-admin enforcement schema >/tmp/capsem-enforcement-schema.json && diff -u schemas/capsem.enforcement-pack.v1.schema.json /tmp/capsem-enforcement-schema.json` passed.
  - `rg -n 'capsem-admin policy|@policy|def policy\(|\[\s*"policy"|capsem\.policy-pack|capsem\.policy-compile|capsem\.policy-backtest|PolicyPackV1|PolicyRuleV1|PolicyDecision|dump_policy_pack|validate_policy_pack|compile_policy_pack|run_policy_backtest|data/enforcement/policy|schemas/capsem.policy-pack|unsupported policy path|policy pack|policy rule|policy packs|policy rules' src tests docs data schemas -S` returned no matches.
  - `cargo test -p capsem-network-engine http_policy --lib`, `cargo test -p capsem-core mcp_frame --lib`, and `cargo test -p capsem-security-engine --lib` passed after narrow internal decision-type renames removed stale `HttpPolicyDecision` / `McpPolicyDecision` names from the transport/security boundary.
  Fix applied during replay: renamed the public admin enforcement-pack surface from `capsem-admin policy` / `capsem.policy-pack.v1` to `capsem-admin enforcement` / `capsem.enforcement-pack.v1`, moved enforcement fixtures under `data/enforcement/packs/`, regenerated the schema artifact, updated docs/tests, and added a negative test proving `policy` is not kept as a public alias.
- 2026-05-23: Continued S18 with the operator observability replay slice.
  Verification commands:
  - `cargo test -p capsem-service handle_logs --bin capsem-service` passed with 2 focused tests, proving structured process Security Engine decisions and canonical resolved Security Events are exposed through `/logs`.
  - `cargo test -p capsem-service handle_debug_report --bin capsem-service` passed, proving `/debug/report` remains pasteable and structured.
  - `cargo test -p capsem-service handle_list_reports_profile_status_for_each_vm --bin capsem-service` passed, proving VM list reports current, needs-update, deprecated, revoked, and corrupted profile states.
  - `cargo test -p capsem-service attach_metrics_snapshot_projects_security_status_fields --bin capsem-service` passed, proving live metrics snapshots project enforcement/detection/security status fields.
  - `cargo test -p capsem status --bin capsem` passed with 35 CLI/status tests, including security-engine debug-report parsing and profile-status list formatting.
  - `cargo test -p capsem format_session_logs --bin capsem` passed with 2 CLI log-formatting tests, proving structured process security lines are preserved and resolved Security Event summaries are added.
  - `cargo test -p capsem logs_response_serde --bin capsem` passed, proving the typed log envelope still round-trips.
  Note: two attempted multi-filter cargo invocations were rejected by cargo syntax and rerun as valid package/test filters above.
- 2026-05-23: Continued S18 with the admin install/package/operator replay slice.
  Verification commands:
  - `uv run python -m pytest tests/capsem-bootstrap/test_dev_setup.py -q` passed with 10 tests and 1 skipped test, proving developer bootstrap exposes and smokes the editable `capsem-admin` entrypoint.
  - `uv run python -m pytest tests/test_package_scripts.py tests/test_repack_deb.py tests/test_verify_deb_payload.py -q` passed with 12 tests and 7 skipped tests, proving package assembly, Debian repack, and payload verification still include the admin CLI layout.
  - `uv run python -m pytest tests/test_release_workflow_policy.py -q` passed with 26 tests, proving release workflow ordering still prepares packaged admin tooling before OS packages and keeps release checks wired.
  - `uv run python -m pytest tests/test_build_assets_script.py tests/test_admin_hygiene.py -q` passed with 6 tests, proving image build scripts route through `uv run capsem-admin image build` and admin workflows keep their typed/Pydantic hygiene checks.
  - `uv run capsem-admin --version` returned `capsem-admin, version 1.1.1778860037`.
  - `uv run capsem-admin profile validate schemas/fixtures/profile-v2-valid.json --json` passed and reported `schema: capsem.profile.v2`, `profile_id: everyday-work`, and `revision: 2026.0520.1`.
  - `uv run capsem-admin settings validate schemas/fixtures/service-settings-v2-complete.json --json` passed and reported `schema: capsem.service-settings.v2`.
  - `uv run capsem-admin enforcement schema >/tmp/capsem-enforcement-schema-smoke.json && diff -u schemas/capsem.enforcement-pack.v1.schema.json /tmp/capsem-enforcement-schema-smoke.json` passed.
  - `uv run capsem-admin enforcement validate data/enforcement/packs/http-google-secret-enforcement.toml --json` passed and reported `schema: capsem.enforcement-pack-validation.v1`.
  - `uv run capsem-admin enforcement compile data/enforcement/packs/http-google-secret-enforcement.toml --json` passed and reported `schema: capsem.enforcement-compile.v1` with `rule_count: 1`.
  - `uv run capsem-admin detection validate data/detection/sigma/google-secret-egress.yml --json` passed and reported `schema: capsem.detection-pack-validation.v1`.
  - `uv run capsem-admin enforcement backtest data/enforcement/packs/http-google-secret-enforcement.toml --events data/policy-context/canonical-policy-contexts.jsonl --json` passed with 4 events, 1 rule, and 1 matched block decision.
  - `uv run capsem-admin detection backtest data/detection/sigma/google-secret-egress.yml --events data/policy-context/canonical-policy-contexts.jsonl --json` passed with 4 events, 1 rule, and 2 findings.
  - `uv run python -m pytest tests/test_security_packs.py tests/test_admin_cli.py -q` passed with 57 tests after tightening validation-report schema assertions.
  - `pnpm --dir docs run build` passed again with 69 pages.
  - `rg -n '/security/policy/|docs/security/policy|security/policy' docs/src/content/docs docs/src -S` returned no matches after the docs route cleanup.
  Fixes applied during replay: split the remaining generic admin validation report into explicit `capsem.enforcement-pack-validation.v1` and `capsem.detection-pack-validation.v1` schemas so `capsem-admin enforcement validate` and `capsem-admin detection validate` expose distinct public contracts; moved the public Rule Authoring page from `/security/policy/` to `/security/rules/` and updated docs links so the public security surface says rules/enforcement/detection instead of policy.
- 2026-05-23: Continued S18 with the profile-backed VM/doctor replay slice.
  Verification commands:
  - `uv run python -m pytest tests/test_profiles.py tests/test_manifest.py tests/test_manifest_check.py tests/test_image_plan.py tests/test_image_verify.py tests/test_doctor.py -q` passed with 127 tests.
  - `cargo test -p capsem-service asset_supervisor --lib` passed with 10 tests.
  - `cargo test -p capsem-service profile_asset --bin capsem-service` passed with 4 tests.
  - `cargo test -p capsem-service vm_profile_pin --bin capsem-service` passed with 5 tests.
  - `cargo test -p capsem-service handle_profile_catalog --bin capsem-service`, `handle_reconcile_profile_catalog`, and `revoked` passed with 2, 3, and 3 focused service tests; `deprecated` matched no focused service-bin tests after the VM list/profile-status coverage already proved deprecated state.
  - `uv run python -m pytest tests/capsem-e2e/test_profile_asset_boot.py -q` passed with 1 test and 1 skipped test.
  - `uv run python -m pytest tests/capsem-e2e/test_winterfell_fork_lineage.py -q` passed with 1 real-VM fork-lineage test.
  - `cargo test -p capsem-process mcp_runtime` passed with 14 runtime tests after adding the Profile V2 HTTP read/write runtime projection regression.
  - `cargo build -p capsem-process -p capsem-service -p capsem` passed so the E2E fixture launched rebuilt binaries.
  - `uv run python -m pytest tests/capsem-e2e/test_e2e_lifecycle.py::TestStartExecDelete::test_start_exec_delete tests/capsem-e2e/test_e2e_lifecycle.py::TestDoctor::test_doctor_passes -q` passed with 2 real-VM tests.
  - `uv run python -m py_compile guest/artifacts/diagnostics/test_mcp.py guest/artifacts/diagnostics/test_network.py && uv run python -m pytest tests/capsem-build-chain/test_pack_initrd.py -q` passed with 4 initrd-pack tests.
  Discovery and fixes during replay:
  - The doctor bundle initially failed because positive MCP probes still assumed `elie.net` was always allowed. The diagnostics now skip those positive probes only when the active profile intentionally blocks network access, while negative/blocking diagnostics still run.
  - The remaining doctor failure was real: `POST https://example.com` reached upstream and returned `405` even with `CAPSEM_WEB_ALLOW_WRITE=0`. Root cause was that Profile V2 derived `http.read` and `http.write` rules existed in VM-effective settings but `capsem-process` only compiled `http.request` callbacks into the runtime Security Engine.
  - The long-term fix compiles `http.read` and `http.write` into guarded CEL over `common.event_type == 'http.request'`, splits read methods from write methods, preserves profile rule priority before catch-all defaults, and places service runtime overlays before profile defaults so live operator blocks win. Release-scope clarification: S15 owns real ask/confirm UX, so shipped Profile V2 `ask` decisions resolve as allow/pass until the confirm resolver lands.
  - Normal `just _pack-initrd` could not run locally because Docker/Colima was unavailable; the local release-gate VM proof repacked the initrd from existing `target/linux-agent/arm64` guest binaries plus updated diagnostics, verified the dev manifest signature, and then proved the booted VM used the refreshed diagnostics.
