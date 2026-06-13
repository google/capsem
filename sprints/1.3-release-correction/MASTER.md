# 1.3 Release Correction Sprint

Status: Active execution. Product-code fixes follow this sprint as the
execution ledger.

## Why This Sprint Exists

The 1.3 branch has the right direction, but the release loop exposed a pattern
we must correct before asking for another manual credential/client run: profile
routes are incomplete, some bootstrap/config paths still drift from the profile
contract, protocol tests are too thin, UI surfaces render guesses, and doctor /
bench / smoke do not yet prove the real VM path. This sprint replaces the messy
hotlist with a controlled correction plan and gates.

Manual AGY/Claude/Codex/OAuth runs are forbidden until the local hermetic gates
prove the same rails without user credentials.

## Absolute Contracts

- Profile is the unit of product truth. A session runs a profile.
- Settings are UI/application settings only. They do not decide profile
  behavior.
- Corp owns locked constraints and reporting endpoints.
- Profile owns assets, VM resources, bootstrap root files, enforcement rules,
  detection files, MCP config, plugin config, and surface availability.
- No `user.toml`, no fallback config, no global profile behavior.
- UI/TUI render route contracts. They do not rename profile data or invent
  states.
- The security rail is one CEL/security-event path with typed events and typed
  rule actions.
- Plugins are configured by profile/corp and report structured status/counters.
- Snapshot is a hermetic subsystem surfaced by routes, not a generic activity
  table.
- Doctor, tests, benchmark, and install all use the same manifest/profile/admin
  path.
- Installer packages contain the app/runtime config/manifest provenance, not VM
  asset blobs.

## Status Table

| Slice | Name | Status | Exit Gate |
| --- | --- | --- | --- |
| S0 | Sprint ledger and release hold | Complete | `MASTER.md`, `plan.md`, and `tracker.md` are coherent and linked from old trackers. |
| S1 | Profile/config authority | Complete | `user.toml` rail burned; profile linter always runs; invalid profiles cannot be materialized. |
| S2 | Materialization/assets/resources | Complete | `code` and `co-work` materialize from `capsem-admin`; assets and VM resources verified end to end. |
| S3 | Route contract and API coverage | Complete | Every UI/TUI-used profile/session/stats route has contract tests for both profiles; no 404/501. |
| S4 | Hermetic protocol lab and recorder | In progress | Local lab covers HTTP/HTTPS/SSE/WS/DNS/MCP/model/OAuth/broker without public services, and every protocol case is a full-chain spec: one stimulus, at least ten assertions across parser, security/CEL, DB ledger, logs, UDS, HTTP routes, status counters, and UI-facing serialization. |
| S5 | Doctor/just/benchmark unification | In progress | `just test` and `just smoke` run doctor/E2E/bench through the hermetic lab, no `--fast` release escape; full doctor now passes in 26.20s wall time versus the prior 104.41s failing public-network run. |
| S6 | CEL/security event correction | Complete | IP/TCP/UDP facts and `valid` booleans are first-party CEL objects; no `security.*` predicates. |
| S7 | Runtime protocol fixes | In progress | AGY/Claude/Codex model, MCP, broker, SSE, and tool-call paths pass full-chain acceptance specs with response text/thinking/tool output, token counts, detection/security rows, route output, and no phantom calls. |
| S8 | UI/TUI contract repair | Complete | Sessions/profiles/settings/stats/plugin/MCP/security/file/process views reflect routes and enums only. |
| S9 | Agent bootstrap repair | In progress | AGY, Claude, Codex, MCP, aliases, and profile root files are packaged from profile-owned bootstrap; fresh-VM runtime proof remains open. |
| S10 | Packaging/install/release gate | In progress | Package payload closed contract, `just install`, status/debug, changelog/docs, and benchmark report pass. |
| S11 | Security boundary cleanup | Complete | `sprints/1.3-security-boundary-cleanup/` proves network engine parses/routes only, every plugin contract is `SecurityEvent -> SecurityEvent`, credential broker handles capture/storage/injection without owning logs, log sanitizer is an independent logging plugin that produces ledger projection, raw credentials cannot reach DB/log/route/UI output, and docs/skills teach the boundary. |

## Release Holds

- Hold: no more real OAuth/client manual testing until S1-S7 local gates pass.
- Hold: do not purge or kill user evidence sessions without explicit approval.
- Hold: no old policy/domain/MCP fallback rails may be reintroduced.
- Hold: no package may include rootfs/initrd/kernel asset blobs.
- Hold: no profile route may return 404/501 from installed UI/TUI surfaces.
- Hold: no S4/S7 protocol slice may close on status-code replay or row-exists
  tests; every protocol needs the full-chain assertion matrix in the tracker.
- Hold: session event writes must stay behind `capsem_logger::DbWriter`; no
  protocol, plugin, security, service, or process path may open an ad-hoc
  SQLite writer or insert event rows directly.
- Hold: project dev skills must live under top-level `skills/` with
  `.codex/skills -> ../skills`; `config/skills/` is profile/product payload
  only.
- Hold: Ironbank is the release ledger for VM/security/network/protocol/broker
  proof. Ironbank lives in `tests/ironbank/`, is authored from public
  contracts only, and cannot use Rust internals, `skip`, `slow`, public
  services, status-only replay, or row-exists checks as proof.
- Hold satisfied for S11: `sprints/1.3-security-boundary-cleanup/` closed with
  runtime bytes and ledger bytes as separate materializations; credential
  broker owns capture/storage/injection, logging plugins own final redaction or
  enrichment inside the security engine before logger handoff, every plugin
  receives and emits only `SecurityEvent`, and the logger has no sanitizer
  fallback path. Remaining release readiness still depends on S4/S5/S7/S8/S10.

## Source Evidence

- Active hotlist: `sprints/1.3-debug-loop/current-hotlist.md`
- Security boundary cleanup: `sprints/1.3-security-boundary-cleanup/`
- Lost surface audit: `sprints/1.3-release-correction/lost-surface-audit.md`
- Ironbank contract: `sprints/1.3-release-correction/IRONBANK.md`
- Historical debug tracker: `sprints/1.3-debug-loop/tracker.md`
- Existing narrow Claude note: `sprints/1.3-claude-mcp-bootstrap/`
- Local baseline confirmed on 2026-06-11: host Ollama is reachable at
  `127.0.0.1:11434`; `/api/tags` reports `gemma4:latest` with completion,
  tools, and thinking capabilities. Use this as the local live backend for
  recorder/smoke tests, routed through Capsem, not as a guest install target.
- Ironbank progress on 2026-06-12: `tests/ironbank/test_model_sdk_ledger.py`
  now proves the local OpenAI-compatible SDK path through a real VM, hermetic
  mock server, credential broker capture and replay/injection, query
  injection, JSON/form request credential capture, OAuth/generic credential
  response capture, model response parsing, native tool call ledger rows, file
  write, security latest route, session DB rows, plugin execution counters,
  profile plugin route telemetry, and raw-secret absence.
- Ironbank progress on 2026-06-13: the current black-box release ledgers run
  together with no skips: `CAPSEM_TEST_PRESERVE_ALWAYS=1 uv run pytest
  tests/ironbank/ -q -s --tb=short` (`6 passed in 49.98s`). This proves the
  model SDK, doctor/security, package-manager, agent bootstrap, and native
  profile MCP ledgers as a suite; it does not close the still-open S4/S5/S7
  streaming/provider matrix, UI, and full `just test` gates.
- Ironbank/MCP progress on 2026-06-13: native profile MCP calls now use the
  same logged MCP JSON-RPC rail as framed guest MCP instead of calling the
  aggregator directly. Focused RED/GREEN coverage proves `capsem_mcp_call`
  writes `mcp_calls`, built-in MCP HTTP `net_events`, and matching
  `mcp.tool_call` security-rule rows through the process `DbWriter`; the same
  proof now lives under `tests/ironbank/test_mcp_profile_ledger.py`.
- Integration gate hardening on 2026-06-12: `scripts/integration_test.py` now
  runs service and VM paths with an isolated credential broker test store and
  bounded model fixture calls. Proof:
  `python3 scripts/integration_test.py --binary target/debug/capsem --assets
  assets` passed 47 ledger checks plus ephemeral proof after reproducing the
  native-keychain hang on authenticated local model traffic.
- Integration gate hardening on 2026-06-12 also covers service startup
  self-idempotence: `_wait_for_service_ready` keeps probing after a clean
  `capsem-service` early exit from a compatible peer-start race and fails only
  on nonzero exits or socket timeout. Proof:
  `uv run python -m pytest tests/test_integration_script_profiles.py -q` and
  `python3 scripts/integration_test.py --binary target/debug/capsem --assets
  assets`.
- Integration gate hardening on 2026-06-12 now isolates each integration
  script invocation under `target/integration-capsem-home-$PID`, with
  `CAPSEM_INTEGRATION_HOME` reserved for explicit debugging. The harness
  creates its run directory before writing `service.pid` and closes the parent
  service-log handle after spawn, preventing stale singleton sockets and file
  descriptor leaks from poisoning the final `just test` integration step.
  Proof: `uv run python -m pytest tests/test_integration_script_profiles.py
  -q` and `python3 scripts/integration_test.py --binary target/debug/capsem
  --assets assets`.
- Integration gate hardening on 2026-06-12 also pins `CAPSEM_RUN_DIR` and
  passes `--uds-path` to `capsem-service`. This closes the full-gate failure
  where inherited run-dir state outranked `CAPSEM_HOME`, sent the service to a
  foreign singleton socket, and left the harness waiting on the wrong UDS.
  Proof: `uv run python -m pytest tests/test_integration_script_profiles.py
  -q` and `python3 scripts/integration_test.py --binary target/debug/capsem
  --assets assets`.
- Package install hardening on 2026-06-13 keeps the closed package payload
  contract while making postinstall hydrate VM assets from the installed
  manifest via `capsem update --assets`. Local dev/corp manifests use
  `manifest-origin.json` as the source asset tree; every copied asset is
  blake3-verified and materialized into the same hash-named layout remote
  downloads use. Proof: `cargo test -p capsem-core copy_missing_local_assets
  -- --nocapture`; `cargo test -p capsem local_manifest_asset_source --
  --nocapture`; `uv run python -m pytest
  tests/capsem-build-chain/test_install_asset_payload.py
  tests/capsem-install/test_installed_layout.py::TestInstalledLayoutContract::test_hash_named_assets_exist
  -q`; `just test-install` passes 39/39 install checks with 22 skips and logs
  `event=assets_hydrated`.
- Bootstrap gate hardening on 2026-06-13 makes `bootstrap.sh` run
  `CI=true pnpm install --frozen-lockfile` in every frontend install branch so
  unattended `just test` cannot stop on pnpm's non-TTY module-purge prompt.
  Proof: `uv run python -m pytest
  tests/capsem-bootstrap/test_dev_setup.py::TestDevSetup::test_bootstrap_pnpm_install_is_noninteractive
  -q`; `sh bootstrap.sh -y` passes with doctor 37 passed / 1 skipped.
- Fork ledger hardening on 2026-06-13 fixes the full-gate
  `test_fork_of_fork` failure where copying only `session.db` produced a
  malformed database when committed rows lived in WAL. `clone_sandbox_state`
  now uses SQLite `VACUUM INTO` and verifies the clone with `quick_check`, so
  forked sessions carry a standalone ledger DB. Proof: `cargo test -p
  capsem-core clone_sandbox_state -- --nocapture`; `uv run python -m pytest
  tests/capsem-mcp/test_fork_images.py::test_fork_of_fork -q`.
- Profile-id trap proof on 2026-06-13: the checked-in profiles were
  temporarily renamed to `mary` and `jane` to flush out hardcoded
  `code`/`co-work` assumptions. Full `just test` passed under those temporary
  ids, including Ironbank, integration, benchmark, Linux package build, and
  install E2E. The profiles were then restored to the shipping `code` and
  `co-work` identities and passed `just _materialize-config`, core profile
  contract tests, the full `capsem-admin` suite, and the focused Python
  profile/build-chain tests before the final shipping-name full gate.
- Apple VZ lifecycle hardening on 2026-06-13: checkpoint files now require an
  fsynced `.complete` marker before service registry state can mark a VM
  suspended or resume from warm checkpoint. Save/restore use exclusive
  host-wide locking, cold starts remain shared, and `just test` separates the
  non-serial `-n 4` canary from serial timing/benchmark probes so benchmark
  numbers measure Capsem rather than sibling VZ contention. Proof: `cargo test
  -p capsem-service startup::tests -- --nocapture`; `cargo test -p
  capsem-service checkpoint -- --nocapture`; `cargo test -p capsem-process
  --no-run`; Python non-serial canary `1418 passed, 71 skipped` in `407.58s`;
  serial timing bucket `11 passed, 1 skipped` in `87.67s`.
- Remote CI drift found on 2026-06-13 after the local final gate: macOS and
  Linux Rust coverage still selected the deleted `capsem-debug-upstream`
  crate, and Python lint still validated retired `config/skills`. The workflow
  now selects only packages present in `cargo metadata` and validates
  top-level `skills/`. Keep S10 open until PR CI is green on the pushed
  branch. Proof: `uv run python -m pytest
  tests/test_release_doctor_contract.py::test_ci_workflow_references_only_live_workspace_packages_and_skills
  tests/test_release_doctor_contract.py::test_mock_server_is_the_only_hermetic_fixture_server_contract
  -q`; focused release guard `25 passed`; `uv run capsem-builder
  validate-skills skills`.
- Linux ARM CI drift found on 2026-06-13 after the workflow fix:
  `capsem-core` KVM checkpoint tests still compiled x86 vCPU/IRQ/PIT/MMIO
  helpers on ARM Linux even though production checkpoint serialization is
  x86_64-only. Header encode/decode tests now stay portable, and the full
  checkpoint serialization tests are gated to x86_64. Keep S10 open until the
  pushed PR CI proves the ARM runner. Local proof: `uv run python -m pytest
  tests/test_release_doctor_contract.py::test_kvm_checkpoint_x86_state_tests_are_arch_gated
  -q`; `cargo check -p capsem-core --tests`; `uv run ruff check
  tests/test_release_doctor_contract.py`; `git diff --check`.
- Second CI drift found on 2026-06-13: macOS coverage compiled `capsem-app`
  before `frontend/dist` existed, and Linux ARM pty-agent exec tests selected
  `/root` as cwd for a non-root runner user because the directory existed.
  The workflow now builds frontend before Rust coverage, and agent exec uses
  `/root` only when running as root. Keep S10 open until pushed CI proves this
  remotely. Local proof: `cargo test -p capsem-agent exec_ -- --nocapture`;
  `cd frontend && CI=true pnpm install --frozen-lockfile && pnpm run build`;
  `cargo check -p capsem-app --tests`; release-doctor workflow guards.

Those files remain evidence. This sprint is the execution authority.
