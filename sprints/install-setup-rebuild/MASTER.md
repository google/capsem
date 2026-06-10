# Meta Sprint: install-setup-rebuild

## Status

| Sprint | Status | Purpose |
| --- | --- | --- |
| T0: Contract and Trace | Done | Install/setup replacement contract is frozen and mapped to owners. |
| T1: Installer Discipline | Done | Local dev asset payloads are packaged before install; `just install` no longer syncs assets afterward. |
| T2: Asset Lifecycle | Done | First-class assets API/CLI, non-blocking startup reconciliation, durable status checkpoints, and UI/service gating are wired. |
| T3: UI States | Done | Setup/onboarding wizard is gone; New Tab owns asset readiness and profile/session creation state. |
| T4: Brokered Substitution Plugin | Done | Core/security backend complete: broker references, substitution logs, `.env`, HTTP header, token-exchange body detection, preview redaction, and typed reader exposure. |
| T5: Burn Setup | Done | `capsem setup`, auto-setup, postinstall setup execution, setup-state, GUI wizard, wizard action, and all `/setup/*` routes are gone. |
| T6: Security Action Materialization | Done | Typed rule actions, action-only default broker rules, imported/Sigma-derived validation, broker capture/substitute action plugins, HTTP post-action materialization, and cross-family runtime DB handoff through the security-engine emitter are in. |

## Release Hold

Active. Do not call install/setup done until:

- Full interactive `just install` must pass on macOS before release sign-off.
- The package owns previous-version replacement. It must stop old Capsem
  processes and remove the old `/Applications/Capsem.app` and package-owned
  share payload before installing the new payload, so downgrade/reinstall works
  without PackageKit version tricks.
- Package builders accept an explicit manifest input. Local dev, CI, and corp
  package builds use the same package rail and choose the manifest with
  `--manifest`; no post-install local asset patching is allowed.
- The service can start without `capsem setup`.
- Assets are independently reconciled and visible through `/assets/status`;
  richer slow-download fixture proof remains part of final install gates.
- The UI can open while assets are not ready and disables session creation
  until assets are explicitly ready.
- VM-observed credentials, including `.env` keys and visible OAuth/token
  exchange material, are brokered by default: raw values are saved through one
  broker credential-store path, user settings store only stable credential
  references, guest config receives only those references, the host MITM/security
  boundary resolves raw secrets only for upstream provider dispatch, downstream
  security/logging sees only references, `session.db` persists them as top-level
  shared security-event fields, and no raw secret is logged.
- Security actions must not remain MITM helper side paths. Rules match through
  CEL/Sigma, plugin-only work uses typed `decision = "action"` rules that cannot
  shadow enforcement verdicts, registered actions run as plugins, plugins
  consume the matched rule and current `SecurityEvent`, plugins return the next
  `SecurityEvent`, and HTTP outbound requests are materialized from the final
  post-action event.
- Parser/runtime paths must instantiate canonical `SecurityEvent`s directly and
  submit them to one auditable emitter. The emitter owns batching, DB writer
  handoff, logging/detection/enforcement fanout, and future multiprocess
  transport; side audit/security table writers are forbidden.
- `just install`, `just test-install`, and `just test` recipe dry-runs pass.
  Live Docker/systemd `just test-install` and live full `just test` now pass;
  live interactive macOS `just install` remains a final release gate.

## Current Diagnosis

The current installer is not a single state machine. It is a chain of side
effects split across `just install`, `.pkg` postinstall, `capsem setup`,
LaunchAgent startup, asset downloads, and UI launch.

The failure mode we just hit is structural:

- `just install` stamps a new dev binary and manifest.
- The package carries only `manifest.json`, not the newly stamped dev assets.
- Previous `.pkg` postinstall ran `capsem setup`.
- Previous `capsem setup` only reconciled assets from the `welcome` step; on
  reinstall that step could already be marked done.
- The service starts with a manifest that names a new hash-prefixed `initrd`
  that has not been copied or downloaded.
- The UI sees a live service but missing assets.
- The outer `just install` tries to patch this after the installer exits by
  syncing local assets, which is too late and not the same path users run.

## Target Contract

Install owns only:

- Lay down app and binaries.
- Lay down package-provided manifest/assets.
- Register service.
- Start service.
- Fail if daemon/gateway cannot become reachable.

Assets own:

- Resolve manifest.
- Download or verify files.
- Expose status/progress.
- Never depend on onboarding state.

UI owns:

- Disconnected service state.
- Asset-progress/missing/failed state.
- Profile creation and user-facing configuration.

Brokered substitution plugin owns:

- Run by default before ordinary security logging, CEL, detection, enforcement,
  UI previews, or session DB writes.
- Detect credential material inside VM context, including `.env` files and
  visible OAuth/token exchanges.
- Persist real credential values into the broker credential store through one
  broker-owned write path; user settings store only references.
- Replace raw credential values with stable credential references before
  security events, logs, policy evaluation previews, or UI/API responses see
  them.
- Support the general substitution contract for future sensitive material
  classes without creating protocol-specific engines.
- Use BLAKE3 as the canonical substitution/fingerprint value for brokered
  credentials.
- Attach the BLAKE3 reference/fingerprint to security events and `session.db`
  rows as a top-level shared security-event field in a protocol-agnostic way
  across HTTP, DNS-derived payloads, MCP, model, file, process, OAuth, GitHub,
  and `.env` observations.
- Log broker actions with provider, source, BLAKE3 reference/fingerprint, and
  outcome, never the secret value.
- Defer ask-before-save, autosave-off, and broker-disable controls until after
  the default broker contract is proven.

`capsem setup` owns nothing in the target architecture.

## Architecture Page

- [Credential Broker Invariant](T4-credential-broker-invariant.md)
- [Security Action Materialization](T6-security-action-materialization.md)

## T6 Slice Ledger

Planned:

- Add typed action identifiers to policy rules and validate them through one
  action registry shared by native CEL and Sigma-derived rules.
- Add typed `decision = "action"` rules for plugin-only matches, and prove they
  do not shadow enforcement decisions.
- Add the security action plugin contract:
  `plugin.apply(rule, SecurityEvent) -> SecurityEvent`.
- Consolidate event emission so parser/runtime paths create canonical
  `SecurityEvent`s and submit them through one auditable emitter.
- Make the emitter the only owner of batching, DB handoff, fanout, and future
  multiprocess transport.
- Execute multiple actions deterministically, each receiving the event returned
  by the previous action.
- Preserve matched Policy V2 HTTP rule configs in hook state and execute their
  registered actions before HTTP upstream materialization.
- Register built-in priority-0 broker substitute rules on the merged runtime
  policy so startup and reload paths share the same action defaults.
- Build HTTP upstream requests from the final post-action event instead of
  direct MITM broker helper side channels.
- Register built-in credential broker capture/substitute action plugins.
- Imported/Sigma-derived policy rules validate through the same typed rule
  contract as native settings JSON: callback family, per-callback CEL fields,
  and action identifiers all fail closed through one registry-backed validator.
- `SecurityEventEngine` owns matched action execution and emits exactly the
  final post-action event for the HTTP runtime path.
- The built-in broker capture action brokers credential observations from the
  security event and returns a reference-only event; substitute remains
  upstream-only through the HTTP materializer.
- Preserve the security/logging event as reference-only while allowing the HTTP
  materializer to resolve raw brokered credentials only for upstream dispatch.
- Audit model, MCP, DNS, file, and process paths so logged/enforced events are
  post-action events and unsupported wire mutation is explicit.
- Runtime DB handoff for HTTP/net, model, MCP, DNS, file, process
  exec/audit/completion, broker substitution, and snapshot rows now goes
  through `capsem_core::security_engine::{emit_security_write,
  emit_security_write_blocking}`. `RuntimeSecurityEventType` is the closed
  emitted row identity contract (`as_str`, `family`, strict parse); CEL rule
  callbacks remain `PolicyCallback`.

Required proof:

```bash
cargo test -p capsem-core --lib security_engine -- --nocapture
cargo test -p capsem-core --lib policy_v2 -- --nocapture
cargo check -p capsem-core
cargo check -p capsem-process
uv run pytest tests/capsem-build-chain/test_install_asset_payload.py -q
cargo bench -p capsem-core --bench security_actions -- --quick
uv run pytest tests/capsem-e2e/test_brokered_ai_credentials.py -q
```

The build-chain pytest includes the burn guard that fails on direct core/process
`DbWriter` `WriteOp` sends outside the security engine.

## T4 Slice Ledger

Implemented:

- Canonical `credential:blake3:<hex>` reference generation in
  `capsem-logger`, domain-separated by `capsem.credential.v1`, provider, and
  raw credential.
- Shared `credential_ref` field on logger event structs and `session.db`
  event tables: `net_events`, `model_calls`, `mcp_calls`, `dns_events`,
  `fs_events`, `exec_events`, and `audit_events`.
- `substitution_events` table for brokered substitution plugin logs:
  material class, source, event type, algorithm, reference, outcome, provider,
  confidence, trace/context metadata.
- SQLite `CHECK` constraints rejecting raw/non-reference values in
  `credential_ref` and `substitution_ref`.
- Event producers that do not observe credentials explicitly set
  `credential_ref = None`; detector-backed paths now carry brokered references.
- `capsem-core::credential_broker` owns credential observations, BLAKE3
  substitution, Keychain-backed raw storage, reference-only user-settings
  writes, substitution logs, HTTP header detection, JSON body token-exchange
  detection, and preview redaction helpers.
- Guest config materialization preserves brokered references as guest-visible
  placeholders. It must not resolve brokered provider refs into raw guest env or
  files.
- MITM request header/query handling preserves brokered references for
  telemetry and resolves them from the broker store only when constructing the
  outbound upstream provider request. Unknown sensitive headers keep the old
  short BLAKE3 hash behavior.
- `TelemetryHook` detects request/response JSON body credentials before
  building `NetEvent` / `ModelCall`, redacts captured previews, writes
  substitution rows, and carries the same `credential_ref` into session DB rows.
- `FsMonitor` brokers small `.env` / `.env.*` files observed in the workspace
  path and records the shared `credential_ref` on file events.
- Typed logger readers surface `credential_ref` on new DBs and use
  compatibility `NULL AS credential_ref` expressions for old read-only session
  fixtures.

Verified:

```bash
cargo test -p capsem-logger -- --nocapture
cargo check -p capsem-core
cargo test -p capsem-core credential_broker -- --nocapture
cargo test -p capsem-core brokered_ -- --nocapture
cargo test -p capsem-core fs_monitor -- --nocapture
cargo test -p capsem-core format_headers -- --nocapture
cargo test -p capsem-core telemetry_hook -- --nocapture
uv run pytest tests/capsem-e2e/test_brokered_ai_credentials.py -q
cargo check -p capsem-process -p capsem-service -p capsem-mcp -p capsem-mcp-builtin
cargo test -p capsem-core --lib --no-run
cargo test -p capsem-process --no-run
git diff --check
```

Still open:

- Existing manually entered raw settings can still materialize as raw guest env
  and must be migrated later into Keychain behind the same broker API.
- Full live provider-token/browser OAuth E2E remains a later integration gate;
  T4 now proves VM no-raw for brokered Claude/Gemini refs and host MITM
  substitution with fake observable material.

## T1 Slice Ledger

Implemented:

- `just install` no longer invokes `scripts/sync-dev-assets.sh` after
  Installer.app or `dpkg` returns.
- macOS and Linux packages always move one selected manifest into the package
  payload. `--manifest` accepts local paths plus `file://`, `http://`, and
  `https://` URLs as the corp/dev override; asset-mode environment variables
  are burned.
- Manifest production is documented and tested through
  `capsem-admin manifest generate <assets_dir>`, including corp custom builds.
  Direct generator internals are not a public package/install path.
- Service/CLI manifest status reports mutable-manifest truth: current hash,
  source provenance, refresh timestamp, validation status/error, and current
  asset/binary versions. It does not pretend the install-time hash is a
  permanent security pin.
- macOS and Linux package scripts write durable install diagnostics to
  `~/.capsem/logs/install.log`, plus per-run timestamped logs and
  `install-latest.log`.
- macOS and Linux postinstall copy any package-provided assets into the
  installed asset directory as part of the package install path.
- Asset copy scripts skip nested directories inside `assets/<arch>/`, so a
  stray nested arch directory cannot abort install.
- Added fast package-contract tests and a reinstall test where only
  `initrd.img`'s hash changes.

Verified:

```bash
uv run pytest tests/capsem-build-chain/test_sync_dev_assets.py tests/capsem-build-chain/test_install_asset_payload.py tests/capsem-build-chain/test_simulate_install_assets.py -q
uv run pytest tests/capsem-install/test_setup_removed.py tests/capsem-install/test_error_paths.py -q
just --dry-run install
just --dry-run test-install
just --dry-run test
```

Still open:

- Full interactive `just install` on macOS. Attempt on 2026-06-06 built release
  binaries, frontend, Tauri app, and `packages/Capsem-1.0.1780763638.pkg` with
  the selected manifest moved by the package rail. It also caught and fixed a release CLI
  exhaustive-match fallout for `ProcessToService::LogFileBoundaryResult`.
  The gate remains open because the second run blocked on the GUI
  Installer.app flow (`open -W packages/Capsem-1.0.1780763638.pkg`) without
  manual completion.

## T5 Slice Ledger

Implemented:

- Deleted `crates/capsem/src/setup.rs` and removed the `capsem setup` CLI
  subcommand.
- Removed first-run auto-setup from CLI session commands.
- Removed macOS `.pkg` and Linux `.deb` postinstall setup execution; installers
  now register/start service and wait for service/gateway readiness only.
- Removed service `/setup/retry`, which spawned setup as a subprocess.
- Removed service `/setup/detect`, which performed host config/credential
  detection and wrote settings.
- Removed frontend setup retry, host-detection calls, setup menu action, and
  GUI onboarding wizard.
- Removed remaining `/setup/state`, `/setup/complete`, `/setup/assets`, and
  `/setup/corp-config` routes. Corporate policy provisioning is now
  `POST /corp-config`.
- Removed the `capsem-core::setup_state` module and the typed
  `rerun_wizard` settings action.
- Replaced legacy setup wizard install tests with a removal regression.
- Replaced service setup compatibility tests with first-class install/assets
  endpoint tests.

Verified:

```bash
cargo check -p capsem -p capsem-service
cargo test -p capsem parse_setup_is_removed -- --nocapture
cargo build -p capsem
uv run pytest tests/capsem-install/test_setup_removed.py -q
uv run pytest tests/capsem-service/test_svc_install.py -q
cargo test -p capsem-core --test settings_spec -- --nocapture
cd frontend && pnpm run check
cargo fmt --check
```

Still open:

- Final install/package gates still need to run with the package discipline
  work.

## T3 Slice Ledger

Implemented:

- `vmStore` reads `/assets/status` directly and stores the richer first-class
  asset status contract instead of relying on the aggregate `/status` asset
  summary.
- New Tab disables customized and quick session creation unless assets are
  explicitly ready.
- New Tab surfaces asset downloading, missing, and reconciliation failure
  details, with an `Ensure` action backed by `/assets/ensure`.
- New Tab normalizes an unreachable asset-status endpoint to `Asset status
  unavailable` instead of leaking an empty API error.
- The old onboarding asset copy and wizard components are deleted.
- Service `/provision` and `/run` enforce the same asset-ready precondition
  and return a clear `412` reason for missing/downloading assets.

Verified:

```bash
cargo test -p capsem-service vm_asset_block_reason -- --nocapture
cargo check -p capsem -p capsem-service
cargo build -p capsem-service
uv run pytest tests/capsem-service/test_svc_install.py -q
cd frontend && pnpm run check
# Browser smoke: loaded http://localhost:5173/ with Chrome DevTools; verified
# New Tab rendered, asset warning was readable, and session buttons were disabled.
```

Still open:

- Richer browser fixtures for downloading/missing/failed asset states.

## T2 Slice Ledger

Implemented:

- Added service `GET /assets/status` for first-class VM asset readiness.
- Added service `POST /assets/ensure`, backed by
  `capsem_core::asset_manager::download_missing_assets` when a manifest is
  loaded. Download failures return status-shaped JSON with an `error` field
  instead of an opaque 500.
- Removed the old `GET /setup/assets` compatibility alias.
- Added CLI `capsem assets status` and `capsem assets ensure`, both with
  `--json`.
- Added frontend API helpers `getAssetsStatus()` and `ensureAssets()`.
- Moved service asset tests to `/assets/status` and `/assets/ensure`.
- Added non-blocking startup asset reconciliation. The daemon starts the same
  single-flight ensure worker used by `/assets/ensure` after creating service
  state, without delaying socket bind or gateway readiness.
- `/assets/status` reports in-memory reconcile details:
  `downloading`, `current_asset`, `bytes_done`, `bytes_total`, `downloaded`,
  and `reconcile_error`.
- Asset reconciliation persists `asset-status.json` beside the service run
  directory. The file records reconcile start, per-asset completion
  checkpoints, and final success/failure. Service startup reloads the file and
  clears stale `in_progress` state so a crashed daemon cannot claim an old
  download is still active.

Verified:

```bash
cargo check -p capsem -p capsem-service
cargo test -p capsem parse_assets -- --nocapture
cargo test -p capsem parse_setup_is_removed -- --nocapture
cargo test -p capsem-service asset_reconcile -- --nocapture
cargo test -p capsem-service ensure_assets -- --nocapture
cargo test -p capsem-service asset_status_reports_reconcile_progress_fields -- --nocapture
cargo build -p capsem-service
uv run pytest tests/capsem-service/test_svc_install.py -q
cd frontend && pnpm run check
cargo fmt --check
```

Still open:

- Live startup "service reachable while slow asset download is still in
  progress" needs a slow release fixture integration test.
- Slow-release fixture proof remains a final install gate.

## Verification Commands

Exact gates will be updated per task, but the sprint cannot close without:

```bash
uv run pytest tests/capsem-install/ -q
uv run pytest tests/capsem-service/test_svc_install.py -q
cargo test -p capsem-core asset_manager credential -- --nocapture
cargo test -p capsem install assets credential -- --nocapture
cd frontend && pnpm run check && pnpm test
just install
just test
```

If a listed command does not exist yet, its task must either create it or the
tracker must replace it with the concrete gate before closure.
