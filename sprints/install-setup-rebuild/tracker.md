# Sprint: install-setup-rebuild

## Tasks

- [x] Create sprint plan, tracker, and master board.
- [x] T0: Freeze install/setup replacement contract.
- [x] T0: Trace current macOS `.pkg`, Linux `.deb`, Docker install test, and `just install` flows.
- [x] T0: Decide local dev asset policy: selected package manifest plus profile
  `file://` URLs for dev assets.
- [x] T1: Remove post-installer mutation from `just install`.
- [x] T1: Make package payload mode explicit and testable.
- [x] T1: Make package install own previous-version removal, including stale
  GUI/service processes and package-owned app/share payload.
- [x] T1: Add explicit package-builder `--manifest` input for local, CI, and
  corp package rails without post-install asset patching.
- [x] T1: Add reinstall test where only `initrd` hash changes.
- [x] T1: Add stale asset symlink regression test.
- [x] T2: Extract asset reconciliation from `capsem setup`.
- [x] T2: Add `capsem assets status` and `capsem assets ensure`.
- [x] T2: Add daemon `/assets/status` and `/assets/ensure`.
- [x] T2: Reconcile assets on service start without blocking daemon availability.
- [x] T2: Record durable asset progress/failure beyond the in-memory service
  status rail.
- [x] T3: Replace setup/onboarding UI states with service/assets/profile states.
- [x] T3: Remove all "run capsem setup" UI copy.
- [x] T3: Disable session creation until assets are ready.
- [x] T3: Simplify/remove AI setup wizard pages; provider setup happens inside the VM.
- [x] T4: Add general brokered substitution plugin contract.
- [x] T4: Add credential observation and brokered-reference contract.
- [x] T4: Add BLAKE3 substitution/reference generation for brokered credentials.
- [x] T4: Add protocol-agnostic top-level credential reference fields to shared security events and session DB writes.
- [x] T4: Detect fake `.env` AI key candidates inside VM/workspace.
- [x] T4: Detect fake OAuth/token exchange material through the security pre-plugin path.
- [x] T4: Detect fake HTTP authorization/GitHub token material through the same broker path.
- [x] T4: Add broker-owned API that saves observed credentials to the broker
  store by default and writes only references to user settings.
- [x] T4: Move brokered raw credential storage to macOS Keychain, with user
  settings storing only `credential:blake3:<hex>` references.
- [x] T4: Keep brokered references in guest config and resolve raw secrets only
  at the host MITM/security upstream boundary.
- [x] T4: Return stable credential references to the security/logging/policy pipeline.
- [x] T4: Record BLAKE3 credential references/fingerprints in broker logs and security events.
- [x] T4: Add substitution log records with material class, source, algorithm, reference, outcome, and context.
- [x] T4: Persist BLAKE3 credential references/fingerprints in `session.db`.
- [x] T4: Prove raw credential logging is test-failing.
- [x] T4: Add broker invariant architecture page.
- [x] T5: Remove `capsem setup` CLI and auto-setup.
- [x] T5: Remove `/setup/retry` and `/setup/detect` routes.
- [x] T5: Remove remaining `/setup/state`, `/setup/complete`,
  `/setup/assets`, and `/setup/corp-config` compatibility endpoints after T2/T3
  replacements land.
- [x] T5: Replace setup tests/docs with install/assets/profile tests/docs.
- [x] T6: Create security action materialization sub-sprint.
- [x] T6: Add typed rule `actions` and validate action identifiers through one
  registry for native policy/CEL rules.
- [x] T6: Add typed plugin-only `decision = "action"` rules so broker/default
  actions can match without becoming enforcement verdicts.
- [x] T6: Prove action-only default rules do not shadow block/ask/rewrite/allow
  enforcement decisions.
- [x] T6: Reject malformed action-only rules that have no actions or carry
  rewrite fields.
- [x] T6: Wire Sigma-derived rule validation through the same action registry.
- [x] T6: Define the security action plugin contract as
  `plugin(rule, SecurityEvent) -> SecurityEvent`.
- [x] T6: Make the HTTP parser/runtime path instantiate canonical
  `SecurityEvent`s directly and submit the post-action event to one auditable
  security-event engine/emitter boundary.
- [x] T6: Make the security-event engine own action execution and post-action
  emission for the HTTP materialization path.
- [x] T6: Extend the same emitter integration across model, MCP, DNS, file,
  and process logging paths without claiming unsupported wire mutation.
- [x] T6: Consolidate DB writer batching/handoff and detection/logging fanout
  behind the emitter for all new security-event rows.
- [x] T6: Prove new non-HTTP audit/security rows cannot bypass the emitter path.
- [x] T6: Prove multiple actions run deterministically and each action receives
  the event returned by the previous action.
- [x] T6: Build the HTTP request security event before actions and materialize
  brokered HTTP upstream credentials from the final post-action event.
- [x] T6: Register built-in credential broker capture/substitute action plugins
  and execute matched HTTP rule actions before brokered HTTP materialization.
- [x] T6: Register default priority-0 broker substitute rules through merged
  runtime settings so startup and reload paths share the same action contract.
- [x] T6: Move credential broker capture/substitute side effects behind the
  built-in action plugins for the security-event action path.
- [x] T6: Remove direct MITM credential substitution helper path from the
  upstream request builder path for brokered HTTP header/query materialization;
  MITM now asks the security engine to materialize an HTTP `SecurityEvent`.
- [x] T6: Keep security/logging/session DB credential views reference-only while
  resolving raw brokered credentials only inside the HTTP materializer for
  upstream dispatch.
- [x] T6: Prove full MITM broker action materialization sends raw only upstream
  while `session.db` stores only the broker reference.
- [x] T6: Audit model, MCP, DNS, file, and process event paths for post-action
  logging/enforcement and explicit unsupported wire-mutation boundaries.
- [x] T6: Add fast benchmarks for rule match + no-op action, broker substitute,
  and short action chains.
- [ ] Verification: macOS `just install` normal package path.
- [x] Verification: Docker/systemd install e2e.
- [x] Verification: full `just test`.
- [x] Verification: dry-run parse of `just install`, `just test-install`, and
  `just test` recipes.

## Notes

- Discovery: `capsem setup` currently reconciles assets only from the
  `welcome` step. A reinstall can copy a new manifest while skipping the
  asset work because `welcome` is already marked complete.
- Discovery: current local `just install` mutates `~/.capsem/assets` after the
  macOS Installer returns. That is not the same path a user runs and races UI
  launch/service start.
- Discovery: setup currently mixes assets, service registration, host
  credential detection, corp config, PATH hints, and onboarding state. These
  need separate owners.
- Decision: service/gateway readiness is required before UI launch.
- Decision: asset readiness is not required before UI launch, but session
  creation must be disabled until assets are ready.
- Decision: AI credential brokering is on by default. It begins from
  VM/user-observed credentials and visible token exchanges, not host scraping
  during install.
- Decision: v1 saves observed raw credentials to the broker credential store
  by default and writes only stable credential references to user settings.
  On macOS the production store is Keychain. Tests use an explicit
  file-backed broker store. Ask-before-save, autosave-off, ignore, and disable
  controls are follow-up product settings.
- Decision: broker substitution values use BLAKE3, formatted as stable
  credential references, so security events and CEL can correlate credential
  use without seeing raw secret material.
- Decision: substitution logging is implemented as a general security
  pre-plugin contract. Credentials are the first material class, but the log
  shape is protocol agnostic and can support future sensitive substitutions.
- Decision: the broker is protocol agnostic. HTTP/GitHub/OAuth/`.env`/model/MCP
  observations all carry the same BLAKE3 credential reference through security
  events, logs, and `session.db`; protocol-specific tables may add context but
  must not invent a second credential identity.
- Decision: credential reference/fingerprint is a top-level shared
  security-event field. Typed protocol payloads/table rows add context only.
- Decision: the VM may receive `credential:blake3:<hex>` placeholders, but it
  must not receive brokered raw provider secrets. Raw secrets are resolved from
  the broker store only by the host security/MITM boundary for the upstream
  provider request.
- Decision: remove the AI setup wizard path instead of adding more UI features.
  Users configure providers in the VM; the broker persists observed credentials.
- Decision: setup execution paths are forbidden. The CLI command, first-run
  auto-setup, installer postinstall setup invocation, server-side setup retry,
  host config detection endpoint, GUI onboarding wizard, setup-state module,
  and all `/setup/*` service routes are removed.
- Decision: `/assets/status` is the first-class asset readiness route.
  `/assets/ensure` reconciles missing/corrupt manifest-backed assets and
  returns status-shaped output even on download failure so UI/CLI can explain
  the problem. `/setup/assets` is removed.
- Decision: service startup triggers one background asset reconciliation using
  the same ensure worker as `/assets/ensure`. It updates in-memory status and
  never blocks daemon socket binding or gateway readiness.
- Decision: asset byte-progress callbacks stay in memory for speed. The
  service persists durable status checkpoints at reconcile start, per-asset
  completion, and final success/failure, and clears stale `in_progress` state
  when a new daemon starts.
- Decision: local dev installs package the selected manifest and materialized
  profile file URLs before the installer runs. The package always moves that
  manifest to the runtime asset directory; there is no asset-mode variable and
  no post-install asset patching.
- Decision: package install is allowed to replace or downgrade. The package
  itself removes the previous app/share payload before installing; PackageKit
  version ordering is not a safety mechanism.
- Decision: package builders shape the authoritative manifest for the package.
  By default it comes from the build assets; corp/dev can override it with
  `--manifest <path>`. The selected manifest is copied into the package payload
  and then into `~/.capsem/assets/manifest.json` by postinstall. It is not a
  post-install side channel.
- Completed slice: macOS package builds now include `pkg-scripts/preinstall`.
  It stops old Capsem service processes, kills stale `capsem-app`, removes the
  old `/Applications/Capsem.app`, and removes package-owned
  `/usr/local/share/capsem` before payload install. Package replacement and
  downgrade no longer depend on PackageKit version ordering.
- Completed slice: `scripts/build-pkg.sh` and `scripts/repack-deb.sh` accept
  `--manifest <path>` as the corp/dev override for the package manifest view,
  and preserve package versions instead of appending build timestamps. Local
  install, Docker install, and CI release workflows now pass the manifest
  explicitly.
- Completed slice: `just install` now builds the package with the explicit
  `--manifest` override and materialized profile `file://` asset descriptors.
  Local dev assets are copied by the normal profile asset reconciliation path
  from the installed profile's `file://` descriptors.
- Completed slice: `/profiles/status` and
  `/profiles/{profile_id}/assets/status` now report the runtime asset manifest
  origin, installed path, BLAKE3 hash, format, refresh policy, current asset
  release, and current binary release.
- Verification: package-only macOS build succeeded for
  `packages/Capsem-1.3.1781035201.pkg`; expanded payload contains
  `Scripts/preinstall`, `Scripts/postinstall`, `assets/manifest.json`,
  `profiles/code.toml`, `profiles/code/enforcement.toml`, and companion
  binaries.
- Completed slice: `capsem-logger` now owns canonical
  `credential:blake3:<hex>` reference generation, shared `credential_ref`
  fields on event tables/structs, and `substitution_events` logging. Current
  producers either provide a brokered reference or explicitly set
  `credential_ref = None`.
- Completed slice: `capsem-core::credential_broker` owns credential
  observations, BLAKE3 substitution, user-settings writes, substitution logs,
  HTTP header detection, JSON body token-exchange detection, and preview
  redaction helpers.
- Completed slice: brokered raw credential storage now uses native macOS
  Keychain via `security-framework`; user settings store only the brokered
  reference. Guest config materialization resolves brokered references back
  through the broker store when injecting API keys/Git credentials.
- Completed slice: MITM request header formatting substitutes recognized
  credentials before telemetry, while unknown sensitive headers keep the old
  short-hash behavior.
- Completed slice: `TelemetryHook` detects request/response JSON body
  credentials before building `NetEvent`/`ModelCall`, redacts captured
  previews, writes substitution events, and carries the same `credential_ref`
  into session DB rows.
- Completed slice: `FsMonitor` brokers small `.env`/`.env.*` files observed in
  the workspace path and records the shared `credential_ref` on file events.
- Completed slice: typed logger readers now surface `credential_ref` for new
  DBs and use `NULL AS credential_ref` compatibility for old read-only fixture
  DBs.
- Completed slice: T5 burn removed `crates/capsem/src/setup.rs`,
  `capsem setup`, first-run auto-setup, package postinstall setup execution,
  `/setup/retry`, `/setup/detect`, frontend retry/detection calls, and legacy
  setup wizard install tests. `capsem setup` now has an explicit CLI parser
  regression.
- Completed slice: T2 asset status now has first-class service routes
  `/assets/status` and `/assets/ensure`, CLI commands `capsem assets status`
  and `capsem assets ensure`, frontend API helpers, and service tests moved
  off `/setup/assets`.
- Completed slice: startup asset reconciliation now runs in the background,
  shares the same single-flight ensure path as `/assets/ensure`, and exposes
  `downloading`, `current_asset`, `bytes_done`, `bytes_total`,
  `downloaded`, and `reconcile_error` through `/assets/status`.
- Completed slice: T2 asset reconciliation now writes
  `asset-status.json` beside the run directory, reloads it on service start,
  resets stale active progress after crashes, and persists final
  success/failure so CLI/UI status survives daemon restarts.
- Completed slice: T3 asset UI now reads the first-class `/assets/status`
  contract through `vmStore`, shows downloading/failure/missing details, uses
  `capsem assets ensure` as the visible recovery command, and disables session
  creation unless assets are explicitly ready.
- Completed slice: browser smoke confirmed the New Tab page renders with
  assets unavailable, shows a readable asset-status warning, and keeps session
  creation buttons disabled.
- Completed slice: service `/provision` and `/run` now enforce the same
  asset-ready precondition as the UI, returning a clear `412` reason instead
  of booting into known-missing VM assets.
- Completed slice: T3/T5 final burn deleted the frontend onboarding store,
  onboarding wizard components, setup-state response types, setup menu action,
  `rerun_wizard` typed settings action, service setup-state module, and all
  remaining `/setup/*` service routes. Corporate policy provisioning now lives
  at `POST /corp-config`.
- Completed slice: T1 package discipline now moves exactly one selected
  manifest into the package payload, installs it into `~/.capsem/assets`, and
  relies on profile `file://`/`https://` descriptors for asset reconciliation.
  `CAPSEM_PKG_ASSET_MODE` and `CAPSEM_DEB_ASSET_MODE` are removed. Packages
  also install `manifest-origin.json`, and service status reports the installed
  manifest path, BLAKE3 hash, origin, source, and package timestamp for
  corp/debug provenance.
- Completed slice: install asset-copy scripts now skip nested directories in
  arch asset folders, preventing a stray `assets/arm64/arm64` directory from
  breaking local installed-layout tests.
- Completed slice: T6 runtime logging now goes through one security-engine
  handoff for HTTP/net, model, MCP, DNS, file, process exec/audit/completion,
  broker substitution, and snapshot rows. `RuntimeSecurityEventType` is the
  closed runtime event identity contract for DB handoff metadata; rule
  callbacks remain `PolicyCallback`.
- Burn pass: `test_security_event_rows_go_through_security_engine_emitter`
  scans `capsem-core` and `capsem-process` and fails on direct `DbWriter`
  `WriteOp` sends outside the security engine.
- T4 core/security backend is complete. Broker status remains in ordinary
  settings/security UI work instead of recreating a setup-like feature path.
- Benchmark: `cargo bench -p capsem-core --bench security_actions
  security_event_runtime -- --sample-size 10` measured runtime event
  classification at ~170 ns/event for HTTP, ~177 ns/event for model, and
  ~171 ns/event for MCP on this Mac.
- Stress note: `capsem-logger` async write and batch/coalescing tests passed in
  the full logger suite. The existing logger stress tests prove raw ignored
  `try_write` can drop events under saturation, so production security-event
  paths now use async `emit_security_write` or explicit sync
  `emit_security_write_blocking`; the burn guard rejects `try_emit` reentry.
- Completed T6 slice: `PolicyRuleConfig` now has typed `actions`, backed by
  `PolicyActionId`; TOML and settings JSON reject unregistered action strings.
- Completed T6 slice: `capsem-core::security_engine` now owns the initial
  canonical `SecurityEvent`, `SecurityActionPlugin`,
  `SecurityActionRegistry`, `SecurityEventEmitter`, and HTTP request
  materializer contracts.
- Completed T6 slice: Policy V2 HTTP request evaluation now separates matched
  `decision = "action"` rules from enforcement decisions, preserving action
  rule configs in hook state without letting broker/default rules shadow
  block/ask/rewrite/allow verdicts.
- Completed T6 slice: merged runtime settings now include built-in priority-0
  broker substitute rules as `decision = "action"` defaults, so daemon startup
  and reload share the same broker materialization contract.
- Completed T6 slice: MITM request construction runs registered built-in
  actions from matched action rules and the matched enforcement rule before
  materializing the upstream HTTP request.
- Completed T6 slice: brokered HTTP header/query raw-secret resolution moved
  from MITM `util` into `credential_broker`, and MITM request construction now
  materializes brokered upstream credentials from an HTTP `SecurityEvent`.
- Completed T6 slice: imported/Sigma-derived policy rules now validate through
  the same typed rule contract as native settings JSON: callback family,
  per-callback CEL fields, and `PolicyActionId` all fail closed through one
  registry-backed validator.
- Completed T6 slice: `SecurityEventEngine` now owns matched action execution
  and emits exactly the final post-action `SecurityEvent`; HTTP MITM
  materialization uses this engine before resolving brokered upstream
  credentials.
- Completed T6 slice: the built-in broker capture action now brokers
  credential observations from the `SecurityEvent`, writes through the broker
  store, and returns a reference-only event; substitute remains upstream-only
  via the HTTP materializer.
- Open T6 gap: existing model, MCP, DNS, file, and process logging/enforcement
  paths still need to be migrated onto the same emitter boundary. They remain
  audited in their current protocol tables, but they are not yet all driven by
  `SecurityEventEngine`.
- Decision: plugins do not receive YAML source/target/provider/replacement
  metadata. The matched rule and the current security event are the contract.
- Decision: the parser/runtime path directly instantiates the canonical
  `SecurityEvent`; one auditable emitter owns batching, DB writer handoff,
  logging/detection/enforcement fanout, and future multiprocess transport.
- Decision: side writers to security/audit tables are forbidden for new events.
- Decision: L1 HTTP can materialize outbound mutations first. L2/L3 model/MCP
  semantic mutation must not claim wire effects until an explicit
  reserializer/materializer exists.
- Completed install fix: GitHub issue #63 is fixed for macOS `.pkg`
  postinstall. The package now adds `~/.capsem/bin` to
  `~/.config/fish/config.fish` with an idempotent `fish_add_path` line.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-logger -- --nocapture` covers BLAKE3
  reference stability/domain separation, schema columns, CHECK constraints,
  typed reader compatibility, and net-event reader roundtrip of
  `credential_ref`.
- Unit/contract: `cargo test -p capsem-core credential_broker -- --nocapture`
  covers `.env`, HTTP header, GitHub token-exchange body detection, BLAKE3
  substitution, redaction, Keychain/test-store writes, settings-only reference
  persistence, and credential-store resolution.
- Unit/contract: `cargo test -p capsem-core
  brokered_api_key_ref_resolves_from_keychain_store_for_guest_env --
  --nocapture` proves guest config materialization resolves a broker reference
  into the raw API key only at injection time, while user settings remain
  reference-only.
- Functional: `capsem-logger` writer test persists `substitution_events` and
  `net_events.credential_ref` with the same BLAKE3 reference.
- Functional: `cargo test -p capsem-core fs_monitor -- --nocapture` proves
  `.env` observations flow through the broker, substitution table, and
  `fs_events.credential_ref`.
- Functional: `cargo test -p capsem-core format_headers -- --nocapture` proves
  HTTP/GitHub credential headers are substituted before telemetry while unknown
  sensitive headers remain short-hashed.
- Functional: `cargo test -p capsem parse_setup_is_removed -- --nocapture`
  proves the removed setup command no longer parses.
- Functional: `cargo check -p capsem -p capsem-service` proves the CLI and
  service compile without setup orchestration, `/setup/retry`, or
  `/setup/detect`.
- Functional: `pnpm run check` in `frontend/` proves the frontend no longer
  references setup retry or host detection APIs and type-checks the first-class
  asset status UI path.
- Functional: `uv run pytest tests/capsem-install/test_setup_removed.py -q`
  proves the installed-layout CLI rejects `capsem setup` and does not write
  setup state/user settings.
- Functional: `uv run pytest tests/capsem-install/test_setup_removed.py
  tests/capsem-install/test_error_paths.py -q` proves removed setup remains
  inert even with corrupt setup-state files and missing asset/error paths.
- Functional: `uv run pytest tests/capsem-build-chain/test_sync_dev_assets.py
  tests/capsem-build-chain/test_install_asset_payload.py
  tests/capsem-build-chain/test_simulate_install_assets.py -q` proves stale
  asset symlinks are removed, package payload modes are wired into
  `just install`, nested asset directories are skipped, and reinstall updates
  the initrd asset when only its hash changes.
- Functional: `cargo test -p capsem parse_assets -- --nocapture` proves the
  new assets CLI subcommands parse.
- Functional: `uv run pytest tests/capsem-service/test_svc_install.py -q`
  proves `/assets/status`, `/assets/ensure`, direct `/corp-config`, and
  removed `/setup/*` route behavior.
- Unit/contract: `cargo test -p capsem-service ensure_assets -- --nocapture`
  proves the service asset ensure worker treats no-manifest dev mode as a
  no-op success, persists the final status record, and rejects concurrent
  reconciliation instead of starting a second download.
- Unit/contract: `cargo test -p capsem-service
  asset_status_reports_reconcile_progress_fields -- --nocapture` proves
  `/assets/status`'s backing status shape exposes in-progress asset name and
  byte counters.
- Unit/contract: `cargo test -p capsem-service asset_reconcile --
  --nocapture` proves durable asset status roundtrips failures and resets stale
  `in_progress` records from a prior crashed daemon.
- Unit/contract: `cargo test -p capsem-service vm_asset_block_reason --
  --nocapture` proves service-side VM start gating blocks missing/downloading
  assets and allows ready assets.
- Unit/contract: `cargo test -p capsem-core --test settings_spec --
  --nocapture` proves settings action fixtures no longer include the removed
  wizard action while the remaining action grammar still roundtrips.
- T6 completed coverage: `cargo test -p capsem-core --lib
  policy_v2_imported_sigma_rule -- --nocapture` proves imported/Sigma-derived
  rules reject unknown actions, invalid callback fields, and family/type
  mismatches through the shared typed policy contract.
- T6 completed coverage: `cargo test -p capsem-core --lib security_engine --
  --nocapture` proves action registry order, capture action broker storage,
  missing-plugin failure, duplicate registration rejection, post-action emitter
  ownership, and brokered HTTP materialization keeping the auditable event
  reference-only.
- T6 completed coverage: `cargo test -p capsem-core policy_v2 --
  --nocapture` proves typed action parsing, unknown action rejection,
  malformed action-only rule rejection, action-only rules do not shadow
  enforcement decisions, built-in broker action rules ride merged runtime
  settings, full MITM broker materialization logs only references, and no
  regression across Policy V2 HTTP/DNS/MCP/model rule suites.
- T6 completed coverage: `cargo test -p capsem-core brokered_ -- --nocapture`
  proves existing brokered guest-env and MITM upstream substitution behavior
  still passes after moving materialization ownership.
- T6 E2E: `uv run pytest tests/capsem-e2e/test_brokered_ai_credentials.py -q`
  passed, proving guest-visible Claude/Gemini refs, no raw guest secrets, CLI
  usability, and `session.db` reference-only logging after a guest curl through
  the MITM.
- T6 performance: `cargo bench -p capsem-core --bench security_actions --
  --quick` completed on this Mac. Quick-mode medians were approximately
  `security_action_rule_match_noop = 569 ns`,
  `security_action_chain_1 = 42 ns`, `security_action_chain_2 = 61 ns`,
  `security_action_chain_4 = 103 ns`, and
  `security_action_broker_substitute_header_ref = 11.1 us`. Quick mode is a
  smoke baseline, not a release-grade benchmark run.
- T6 completed coverage: `cargo check -p capsem-core` proves the new
  security-engine module and broker helper move compile in non-test builds.
- T6 remaining coverage debt: model, MCP, DNS, file, and process event paths
  still need explicit emitter-boundary migration/proof. HTTP request
  materialization and brokered credential handling are proven through the
  security-event engine.
- UI/visual: Chrome DevTools smoke at `http://localhost:5173/` proved the New
  Tab page renders, shows `Asset status unavailable` when the first-class
  assets endpoint is unreachable, and disables `Customize Session...` /
  `Quick Session` until assets are ready.
- Static gate: `just --dry-run install` proves the install recipe parses and
  assembles packages with the selected manifest and no post-installer asset
  sync.
- Static gate: `just --dry-run test-install` proves the Docker/systemd install
  recipe still expands cleanly after the package/install refactor.
- Static gate: `just --dry-run test` proves the full release test recipe still
  expands cleanly across audits, coverage, Python suites, VM scripts,
  benchmarks, cross-compile, and Docker install stages.
- Adversarial: raw credential strings are rejected by `credential_ref` /
  `substitution_ref` schema checks and asserted absent from substitution/log DB
  rows in logger, file-monitor, and telemetry-hook tests.
- E2E/VM or integration: `uv run pytest
  tests/capsem-e2e/test_brokered_ai_credentials.py -q` boots a VM with
  brokered Claude/Gemini refs, proves guest env/config contains refs but not
  raw secrets, runs both CLI help paths, and verifies session DB records the
  Anthropic broker ref without logging the raw secret.
- Telemetry/observability: `cargo test -p capsem-core telemetry_hook --
  --nocapture` proves header/body observations create substitution rows, redact
  previews, and carry shared `credential_ref` into `net_events`/`model_calls`.
- Performance: pending.
- Missing/deferred: existing manually entered raw settings are still accepted
  for compatibility and can still materialize as raw guest env. A later
  migration must move those raw values into Keychain and replace them with
  broker references before claiming the stronger invariant for legacy/manual
  settings.
- Missing/deferred: richer browser fixtures for downloading/missing/failed
  asset states remain useful, but the unavailable-state smoke is covered.
- Missing/deferred: live startup "service reachable while slow asset download
  is still in progress" needs an integration harness with a deliberately slow
  release fixture.
- Hard-test finding: full `just test` caught a stale CLI parity expectation
  for removed `capsem setup`; `tests/capsem-mcp/test_cli_parity.py` now rejects
  only current CLI-only commands and the focused parity suite passes.
- Hard-test finding: full `just test` caught Linux `-D warnings` fallout in the
  install Docker build; `KEYCHAIN_SERVICE` is now macOS-only so Keychain-backed
  broker code compiles cleanly on Linux.
- Verification: live Docker/systemd `just test-install` passed with `30 passed,
  26 skipped`.
- Verification: live full `just test` passed end to end after the fixes. The
  gate covered frontend checks/tests/build, clippy, Rust coverage, Python
  integration, injection, VM integration, benchmark, cross-compile, and
  Docker/systemd install e2e.
- Verification note: a macOS Keychain prompt was observed during manual
  monitoring, but the current broker unit/e2e/bench paths that exercise fake
  credentials set `CAPSEM_CREDENTIAL_BROKER_TEST_STORE`; no current hard-test
  path has been confirmed to require native Keychain.
- Missing/deferred: full interactive `just install` on macOS still needs manual
  Installer.app completion before release sign-off. Attempt on 2026-06-06:
  release binaries, frontend, Tauri app bundle, and
  `packages/Capsem-1.0.1780763638.pkg` built successfully with package-owned
  dev asset metadata. The first attempt caught a real release CLI compile
  fallout from the new `ProcessToService::LogFileBoundaryResult` variant; fixed
  by making `capsem shell` ignore that internal response. The second attempt
  blocked for ~8 minutes on `open -W packages/Capsem-1.0.1780763638.pkg`
  waiting for the GUI Installer.app flow, so the terminal recipe was terminated
  cleanly and this gate remains open.
