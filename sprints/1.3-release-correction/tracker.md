# Sprint: 1.3 Release Correction

## Current Rule

No new AGY/Claude/Codex/OAuth manual run until the local due-diligence gates
below pass. Manual credentials are not the debugger.

Security boundary cleanup is now split into
`sprints/1.3-security-boundary-cleanup/` and blocks any claim that credential
broker or model/client traffic is release-ready. The release contract is:
network engine parses/routes only; security engine owns rules/plugins/decisions;
credential broker handles runtime capture/store/injection as a pre-plugin; log
sanitizer is the final plugin before logger materialization; raw credentials
must never reach session DB, route JSON, structured logs, or frontend stats.

Ironbank is the black-box release ledger under `tests/ironbank/`. For VM,
security, network, protocol, credential broker, package-manager, doctor,
benchmark, and release-gate behavior, Ironbank proof must be authored from
public contracts and observed outputs only. Do not inspect Rust/product
internals to decide expected behavior. No `skip`, `skipif`, `slow`, optional
marker, public network, status-code-only replay, or row-exists proof can close
an Ironbank task.

Commit discipline is part of the gate: one fixed bug or functional slice gets
its focused verification and its own commit before the next bug starts. Do not
batch unrelated fixes, do not leave a solved bug uncommitted while opening the
next one, and stage only the files for that slice.

## S0. Sprint Ledger and Release Hold

- [x] Create `sprints/1.3-release-correction/`.
- [x] Create `MASTER.md`, `plan.md`, and `tracker.md`.
- [x] Add guardrail notes to older debug/bootstrap trackers pointing here.
- [x] Snapshot branch and dirty tree before code changes.
  - Branch: `release/1.3-cleanup-pr-v2`.
  - Dirty tree already existed with code/config/test/docs/benchmark changes;
    this sprint creation added/updated sprint docs only.
- [x] Confirm no implementation starts before S0 tracker is coherent.
- [x] Audit lost project surfaces against `origin/main` after discovering
  top-level dev skills were missing from this branch.
  - Finding: `92fa3bd2` created top-level `skills/`; `5489ff10` moved the
    dev skill library into `config/skills/`, which violates the contract.
    `config/skills/` is profile/product payload, not project Codex/dev-agent
    operating manual.
  - Finding: `origin/main` has `.codex/skills -> ../skills`; this branch did
    not preserve it.
  - Evidence: `sprints/1.3-release-correction/lost-surface-audit.md`.
  - Correction in progress: restore top-level `skills/`, restore
    `.codex/skills`, add `/ironbank`, and keep `config/skills/` out of dev
    agent instruction flow.

## S1. Profile/Config Authority

- [x] RED: test that any read/write/use of `user.toml`, `CAPSEM_USER_CONFIG`,
  `user_config_path`, or `load_settings_files` fails the contract.
- [x] GREEN: remove the legacy user config rail from service/runtime/broker/MCP
  tests/benchmarks/helpers.
  - 2026-06-13 follow-up: `capsem-process` now loads runtime rules, plugins,
    network policy, MCP, and model endpoints from the materialized
    `--profile-dir` plus service-written `runtime-overlay.toml`; the built-in
    MCP server requires `CAPSEM_PROFILE_DIR` and compiles its security
    rules/plugins from that same profile directory instead of calling
    settings/corp loaders.
  - Proof: `cargo test -p capsem-process runtime_config -- --nocapture`;
    `cargo test -p capsem-service runtime_profile -- --nocapture`;
    `cargo test -p capsem-mcp-builtin --no-run`; `cargo check -p
    capsem-process -p capsem-mcp-builtin`; and `cargo test -p capsem-process
    --no-run`.
- [x] RED/GREEN: prove old behavior-owned settings were not merely renamed to
  `settings.toml`; profile behavior belongs under profile files and settings
  remains UI/application preferences only.
- [x] RED: malformed corp/settings/profile/rules/detection/MCP/plugin/assets
  files fail through the always-on admin/materialization path.
- [x] GREEN: implement fast always-on profile/config linter in `capsem-admin`
  path, not as optional theater.
- [x] RED/GREEN: profile/admin creation cannot emit invalid profile artifacts.
- [x] Proof: linter covers corp, settings, profile catalog, profile files,
  rules, detection YAML, MCP config, plugins, assets, manifest, OBOM pins, and
  bootstrap root files.
  - 2026-06-11 progress: `capsem-admin profile check` now verifies copied
    workspace profiles with the same strict payload/hash/root-manifest rail as
    source profiles, rejects malformed pinned `mcp.json` even when its
    BLAKE3/size match, and rejects empty pinned package files through the same
    parser used by image workspace generation. Remaining S1 work: make
    any still-missing generated config surfaces equally explicit before closing
    this checklist.
  - 2026-06-11 progress: `capsem-admin` now has a config-root check that loads
    `settings.toml`, typed `corp.toml`, every profile catalog entry, external
    corp enforcement/Sigma rule files, and every pinned profile payload before
    materializing runtime config or image workspaces. It rejects profile
    catalog id mismatch and caught/fixed the stale corp `refresh_interval_hours`
    TOML contract.
  - 2026-06-12 progress: config source layout is explicit and documented in
    `config/README.md` and `tests/README.md`: admin settings artifacts live in
    `config/admin`, corp contracts in `config/corp`, profile source ledgers in
    `config/profiles`, generated runtime config in `target/config`, and test
    fixtures in `tests/fixtures`. Source profiles no longer carry generated
    `hash`/`size` pins; `capsem-admin profile validate/check` rejects source
    pins, while `capsem-admin profile materialize` writes resolved asset and
    profile-file pins into the materialized runtime profile.
  - Proof: `cargo test -p capsem-admin`; `cargo test -p capsem-core
    profile_contract`; `uv run python -m pytest
    tests/capsem-build-chain/test_source_profiles_unpinned.py
    tests/test_config.py tests/test_skills.py`; `uv run ruff check
    scripts/generate_schema.py src/capsem/builder/config.py
    tests/test_config.py tests/test_skills.py
    tests/capsem-build-chain/test_source_profiles_unpinned.py`.
  - Gate wiring proof: `just test` runs root `bootstrap.sh`, validates project
    skills/site shape, and reaches `_materialize-config`; both `just test` and
    `just smoke` materialize every checked-in profile through
    `capsem-admin profile materialize`, so source profile `hash`/`size` pins
    fail the normal release gates instead of only a one-off linter.

## S2. Materialization, Assets, VM Resources

- [x] RED: `just _materialize-config` must materialize every checked-in profile
  and fail if `code` clobbers `co-work`.
- [x] GREEN: `capsem-admin` materializes `code` and `co-work` with current
  `file://` EROFS/LZ4HC assets and matching BLAKE3 hashes.
- [x] RED: package/profile tests fail if profile VM resource fields do not
  propagate to session creation.
- [x] GREEN: new session rootfs image logical size matches
  `profile.vm.scratch_disk_size_gb`.
  - Proof: `uv run python -m pytest tests/test_build_assets_profile.py -q`;
    `just _materialize-config`; generated `target/config/profiles/{code,co-work}/profile.toml`
    points at current `file://` arm64 EROFS assets with manifest BLAKE3 hashes.
  - 2026-06-11 progress: `_materialize-config` now cleans `target/config` once
    and materializes every checked-in `config/profiles/*/profile.toml` through
    `capsem-admin`; it no longer hard-codes `code` or clobbers `co-work`.
  - Proof: `uv run python -m pytest tests/test_build_assets_profile.py -q`;
    `just _materialize-config`; `target/config/profiles/{code,co-work}` both
    contain `profile.toml`, rule files, MCP config, root manifest, package
    lists, and tips, with current arm64 `file://` VM assets.
  - Proof: `cargo test -p capsem-process -- --nocapture`; includes
    `prepare_session_layout_uses_requested_scratch_disk_size` proving a 64 GiB
    sparse `rootfs.img` logical size from the process layout rail.
  - Proof: `cargo test -p capsem-service provision_ -- --nocapture`; `cargo
    test -p capsem-service profile_vm_resources_drive_new_session_defaults --
    --nocapture`; `cargo check -p capsem-service -p capsem-process`.
- [x] RED/GREEN: doctor/status/debug report guest `df -h`, `df -i`, `/dev/vdb`,
  overlay mount options, host sparse-image logical/physical size, and host free
  space.
  - Proof: `cargo test -p capsem-service storage_diagnostics -- --nocapture`;
    `cargo check -p capsem-service`; `uv run python -m py_compile
    guest/artifacts/diagnostics/test_virtiofs.py`.
  - Runtime route contract: `/vms/{id}/info` and `/vms/{id}/status` expose
    `storage.rootfs_image_{logical,physical}_bytes`,
    `storage.host_{total,free,available}_bytes`, and the guest overlay
    identity `/dev/vdb` mounted at `/`.
  - Doctor contract: guest diagnostics now collect `df -h`, `df -i`, and
    `/proc/mounts` overlay mount options alongside the existing `/dev/vdb`
    ext4 probe.
- [x] RED/GREEN: bounded write/install probes cover `/usr/local`,
  `/var/cache/apt`, `/tmp`, `/var/tmp`, and `/root`.
  - Proof: `uv run python -m py_compile
    guest/artifacts/diagnostics/test_storage_write_probes.py`; `(cd
    guest/artifacts/diagnostics && uv run python -m pytest --collect-only
    test_storage_write_probes.py -q)`.
  - Doctor contract: bounded create/read/delete probes cover `/usr/local`,
    `/var/cache/apt`, `/tmp`, `/var/tmp`, and `/root`; `_apt` must be able to
    write `/var/cache/apt/archives/partial` so apt does not fall back to
    unsandboxed root downloads.
- [x] RED/GREEN: Ironbank package-manager probes prove installed packages
  function through apt, npm, uv, pip, and node rails.
  - Required proof: binary presence, version/hash where relevant, and an
    execution that demonstrates the installed package does its intended work.
  - Apt example: install `zstd`, compress known bytes, decompress, compare
    exact output, and inspect logs/DB/routes/status evidence for the VM path.
  - Python/uv/pip example: install a tiny dependency, import it, execute a
    deterministic behavior, and prove no package path needed public fallback.
  - Node/npm example: install/run a tiny CLI/module and prove stdout/exit code
    plus ledger evidence, not just `npm list`.
  - Proof: `uv run python -m pytest tests/ironbank/test_package_managers.py -q
    -s` boots a VM through `/vms/create`, uploads a probe through
    `/vms/{id}/files/content`, runs it through `/vms/{id}/exec`, proves local
    apt/npm/uv/pip/node packages function, and verifies `/status`, `/history`,
    `/history/counts`, plus `exec_events` and `fs_events` ledger fields.
  - Fresh proof after S4/S5 mock-server/DNS/doctor hardening:
    `CAPSEM_TEST_PRESERVE_ALWAYS=1 uv run python -m pytest
    tests/ironbank/test_package_managers.py::test_package_managers_pay_their_ledger_debt_blackbox
    -q -s` (`1 passed in 2.73s`).
- [x] RED/GREEN: integration model fixture must not touch the developer's
  native credential store or hang on a broker/model regression.
  - Root cause: `scripts/integration_test.py` did not set
    `CAPSEM_CREDENTIAL_BROKER_TEST_STORE`, so the model POST carrying an
    Authorization header could hit the native macOS Keychain during a
    black-box VM gate. The request reached MITM but never emitted
    `net_events`, `model_calls`, or `substitution_events`.
  - Fix: integration service and `capsem run` both inherit an isolated
    file-backed broker store under `target/integration-capsem-home/run/`, the
    model curl has explicit connect/total timeouts, and the VM command fails
    closed if the model fixture output file is missing.
  - Proof: `uv run python -m pytest tests/test_integration_script_profiles.py
    -q`; `python3 scripts/integration_test.py --binary target/debug/capsem
    --assets assets` passed with 47 integration ledger checks, local
    OpenAI-compatible model response text, model/tool rows, and ephemeral
    proof.
- [x] RED/GREEN: integration service startup honors `capsem-service`
  self-idempotent startup instead of failing on a clean early exit.
  - Root cause: after the parallel Python gate, a compatible service startup
    race can make one `capsem-service --foreground` process exit `0` while the
    peer owns the socket. `scripts/integration_test.py` treated any early exit
    as fatal before giving the UDS `/list` probe the full readiness window.
  - Fix: `_wait_for_service_ready` keeps probing after a clean `0` exit and
    still fails immediately on nonzero service exits.
  - Proof: RED/GREEN `uv run python -m pytest
    tests/test_integration_script_profiles.py::test_service_ready_wait_accepts_zero_exit_peer_startup
    -q`; `uv run python -m pytest tests/test_integration_script_profiles.py
    -q`; `python3 scripts/integration_test.py --binary target/debug/capsem
    --assets assets` passed with 47 ledger checks and ephemeral proof.
- [x] RED/GREEN: integration harness owns a per-invocation CAPSEM_HOME instead
  of reusing a stale fixed UDS path across focused/full gates.
  - Root cause: the first self-idempotence fix still allowed a fixed
    `target/integration-capsem-home` to race a previous compatible service
    that exited cleanly before this harness could observe `/list`; the failure
    had an empty `target/integration-test-service.log` because the process
    returned before the test-owned service produced child output.
  - Fix: `scripts/integration_test.py` now defaults to
    `target/integration-capsem-home-$PID`, honors
    `CAPSEM_INTEGRATION_HOME` only as an explicit debug override, creates the
    run directory before writing `service.pid`, and closes the parent copy of
    the service log handle immediately after `Popen`.
  - Proof: RED/GREEN `uv run python -m pytest
    tests/test_integration_script_profiles.py -q`; `python3
    scripts/integration_test.py --binary target/debug/capsem --assets assets`
    passed with 47 ledger checks and ephemeral proof from a process-scoped
    integration home.
- [x] RED/GREEN: integration harness pins `CAPSEM_RUN_DIR` and the service UDS
  path so inherited test env cannot redirect service startup.
  - Root cause: `CAPSEM_RUN_DIR` has higher precedence than `CAPSEM_HOME`.
    Under the full `just test` environment, a foreign inherited run dir could
    make `capsem-service` probe a different socket, clean-exit as a compatible
    singleton, and leave the harness waiting on
    `target/integration-capsem-home-$PID/run/service.sock`.
  - Fix: service launch, `capsem run`, and persistence checks all inherit
    `CAPSEM_RUN_DIR=$CAPSEM_INTEGRATION_HOME/run`; service launch also passes
    `--uds-path` with the exact socket the readiness probe uses.
  - Proof: RED/GREEN `uv run python -m pytest
    tests/test_integration_script_profiles.py -q`; `python3
    scripts/integration_test.py --binary target/debug/capsem --assets assets`
    passed with 47 ledger checks and ephemeral proof from the pinned
    runtime directory.

## S3. Route Contract and API Coverage

- [x] Inventory every UI/TUI/service route in one contract doc.
  - Contract doc: `docs/src/content/docs/architecture/service-api.md`.
  - Scope: service-global, profile-scoped, and session-scoped routes are
    separated; verb discipline for `info`, `status`, `list`, `latest`,
    `evaluate`, `edit`, `reload`, and `ensure` is explicit; UI/TUI route rules
    forbid invented names and fallback paths.
- [x] RED: route test fails for missing profile overview/enforcement/detection
  /plugins/MCP/assets route for `code` and `co-work`.
- [x] GREEN: implement routes with no 404/501 for declared UI/TUI surfaces.
  - Proof: `cargo test -p capsem-service
    profile_ui_route_matrix_is_registered_for_all_profiles -- --nocapture`;
    `cargo check -p capsem-service`.
  - The router-level test exercises checked-in profile ids `code` and
    `co-work` across profile overview, assets, enforcement, detection,
    plugins, credential broker detail, MCP, and skills info/list routes.
  - 2026-06-11 progress: gateway route matrix now explicitly forwards
    `/profiles/{profile_id}/plugins/credential_broker/credentials/info`;
    this caught the UI-visible profile 404 path as a gateway route-table gap,
    not a frontend fallback.
  - Proof: `cargo test -p capsem-gateway
    gateway_security_routes_are_explicitly_forwarded -- --nocapture`; `cargo
    test -p capsem-service
    profile_ui_route_matrix_is_registered_for_all_profiles -- --nocapture`;
    `pnpm --dir frontend test src/lib/__tests__/api.test.ts`; `cargo check -p
    capsem-gateway`.
- [x] RED/GREEN: mutation routes either persist via profile object or do not
  exist; no fake success.
  - 2026-06-11 progress: MCP server edit/delete are no longer mounted 501
    stubs. They now mutate through `Profile::upsert_mcp_server` /
    `Profile::delete_mcp_server`, persist `profile.toml`, update MCP
    permission resolution for profile-owned manual servers, and write
    `profile_mutation_events`.
  - Proof: `cargo test -p capsem-core
    profile_mcp_server_mutation_persists_profile_toml_and_permissions --
    --nocapture`; `cargo test -p capsem-service
    profile_mcp_server_edit_delete_persist_profile_and_mutation_ledger --
    --nocapture`; `cargo check -p capsem-core -p capsem-service`.
  - Historical note: at this checkpoint, profile create/edit/delete/clone,
    profile skill add/edit/delete, VM edit, VM restart, and VM reload-profile
    still needed either persistence through the contract object or unmounting.
  - 2026-06-11 progress: profile skill add/edit/delete are no longer mounted
    501 stubs. They now mutate through `Profile::add_skill_path`,
    `Profile::edit_skill_path`, and `Profile::delete_skill`, persist
    `profile.toml`, derive route-visible ids from skill paths, and write
    `profile_mutation_events`.
  - Proof: `cargo test -p capsem-core
    profile_skill_mutations_persist_profile_toml -- --nocapture`; `cargo test
    -p capsem-service
    profile_skills_routes_persist_profile_and_mutation_ledger -- --nocapture`;
    `cargo check -p capsem-core -p capsem-service`.
  - 2026-06-11 progress: profile assets edit is not a route anymore. Asset
    references are materialized by capsem-admin/profile manifests; the runtime
    API exposes assets through status/info/ensure only until there is a typed
    profile mutation contract.
  - Proof: `cargo test -p capsem-service
    profile_assets_edit_route_is_not_mounted -- --nocapture`; `cargo test -p
    capsem-gateway gateway_profile_assets_edit_is_not_forwarded --
    --nocapture`; `cargo test -p capsem-gateway
    gateway_security_routes_are_explicitly_forwarded -- --nocapture`; `cargo
    check -p capsem-service -p capsem-gateway`; `pnpm --dir frontend test
    src/lib/__tests__/api.test.ts`; `pnpm --dir docs build`.
  - 2026-06-11 progress: profile lifecycle write routes
    `create|edit|delete|clone` are unmounted rather than fake 501 contracts.
    Profile lifecycle authoring remains capsem-admin/materialized profile
    files until a typed runtime mutation contract exists.
  - Proof: `cargo test -p capsem-service
    profile_lifecycle_write_routes_are_not_mounted -- --nocapture`; `cargo
    test -p capsem-gateway
    gateway_profile_lifecycle_writes_are_not_forwarded -- --nocapture`;
    `cargo test -p capsem-gateway
    gateway_security_routes_are_explicitly_forwarded -- --nocapture`; `cargo
    check -p capsem-service -p capsem-gateway`; `pnpm --dir frontend test
    src/lib/__tests__/api.test.ts`; `pnpm --dir docs build`.
  - 2026-06-11 progress: VM mutation routes `edit|restart|reload-profile`
    are unmounted rather than fake 501 contracts. VM mutation returns only
    when it persists state or performs a real operation.
  - Proof: `cargo test -p capsem-service
    fake_vm_mutation_routes_are_not_mounted -- --nocapture`; `cargo test -p
    capsem-gateway gateway_fake_vm_mutation_routes_are_not_forwarded --
    --nocapture`; `cargo test -p capsem-gateway
    gateway_security_routes_are_explicitly_forwarded -- --nocapture`; `cargo
    check -p capsem-service -p capsem-gateway`; `pnpm --dir docs build`.
  - Remaining mounted mutation stubs after VM route burn: none known in S3.
- [x] RED/GREEN: session state enum controls available actions for running,
  stopped, incompatible, defunct, paused, and deleted sessions.
  - 2026-06-11 progress: service `/vms/list` and `/vms/{id}/status` now emit
    `available_actions` from the typed VM lifecycle state, gateway preserves
    that contract, and the UI gates row opening/menu actions from the backend
    action list instead of guessing from status text. Incompatible and defunct
    sessions expose only `delete`.
  - Proof: `cargo test -p capsem-service
    vm_lifecycle_available_actions_are_contractual -- --nocapture`; `cargo
    test -p capsem-gateway fetch_status_preserves_session_available_actions --
    --nocapture`; `pnpm --dir frontend test
    src/lib/__tests__/vm-actions.test.ts`; `cargo test -p capsem-service
    handle_list_marks_profile_payload_drift_incompatible -- --nocapture`;
    `cargo test -p capsem-service
    handle_info_marks_profile_payload_drift_incompatible -- --nocapture`;
    `cargo test -p capsem-gateway status_response_serializes -- --nocapture`;
    `pnpm --dir frontend test src/lib/__tests__/api.test.ts`; `cargo check -p
    capsem-service -p capsem-gateway`; `pnpm --dir frontend check`.
- [x] Proof: profile routes are scoped by profile id; service-global routes are
  only service/runtime summaries.
  - Proof: `cargo test -p capsem-service
    mounted_read_routes_reflect_profile_settings_corp_mcp_and_assets_contracts
    -- --nocapture`; `cargo test -p capsem-service
    mounted_mcp_routes_are_profile_scoped_mechanics_only -- --nocapture`;
    `cargo test -p capsem-gateway
    gateway_does_not_forward_retired_mcp_policy_route -- --nocapture`; `cargo
    test -p capsem-gateway gateway_does_not_forward_retired_plugin_authoring_routes
    -- --nocapture`.

## S4. Hermetic Protocol Lab and Recorder

- [x] RED/GREEN: integration tests fail if protocol paths hit public services.
  - 2026-06-12 progress: `scripts/integration_test.py` no longer reads
    `GEMINI_API_KEY`, `GOOGLE_API_KEY`, `settings.toml` credentials, or
    `googleapis.com` live provider traffic. The model proof is now a
    deterministic local OpenAI-compatible request to
    `capsem-mock-server` `/v1/chat/completions`, and DB verification checks
    the resulting `model_calls` row directly.
  - Proof: `uv run python -m pytest tests/test_release_doctor_contract.py -q`
    (`9 passed`); `uv run ruff check scripts/integration_test.py
    tests/test_release_doctor_contract.py`; `python3 -m py_compile
    scripts/mock_server.py scripts/doctor_session_test.py
    scripts/integration_test.py`; `rg -n
    "GEMINI_API_KEY|GOOGLE_API_KEY|googleapis\\.com|include_gemini_probe|expect_model_calls"
    scripts/integration_test.py` is quiet.
- [x] GREEN: one local protocol lab serves HTTP, HTTPS/MITM, DNS, SSE,
  WebSocket, MCP JSON-RPC, OAuth/OIDC, and model fixture replay.
  - 2026-06-12 progress: the shared mock server now serves protocol-shaped
    OAuth authorize/token fixtures and MCP JSON-RPC fixtures alongside the
    existing HTTP/gzip/SSE/WebSocket/OpenAI-compatible model fixtures. The
    token endpoint deliberately emits `capsem_test_*` secret-shaped values so
    broker/recorder tests can prove capture and sanitization without touching
    real credentials.
  - 2026-06-12 correction: the Rust `capsem-mock-server` crate was removed.
    The single fixture implementation is now `scripts/mock_server_runtime.py`,
    launched by `scripts/mock_server.py`; `capsem doctor`, recorder,
    integration, benchmark, and Ironbank tests all use that same runtime.
    `tests/test_release_doctor_contract.py` rejects a restored Rust fixture
    crate or CLI dependency.
  - 2026-06-13 progress: the shared Python runtime now serves deterministic
    DNS A-record fixtures over both UDP and TCP and exposes `dns_udp_addr`,
    `dns_tcp_addr`, and fixture names in the same ready JSON used by recorder,
    doctor, benchmark, and Ironbank callers. This removes the last need for a
    separate local DNS fixture server.
  - Proof: RED `uv run python -m pytest
    tests/test_mock_server_launcher.py::test_mock_server_serves_dns_udp_fixture
    -q` failed before `dns_udp_addr` existed; GREEN `uv run python -m pytest
    tests/test_release_doctor_contract.py tests/test_mock_server_launcher.py
    tests/test_protocol_fixture_recorder.py -q`; `uv run ruff check
    scripts/mock_server_runtime.py tests/test_mock_server_launcher.py
    tests/test_protocol_fixture_recorder.py`; `python3 -m py_compile
    scripts/mock_server_runtime.py scripts/mock_server.py
    scripts/protocol_fixture_recorder.py`.
  - 2026-06-13 progress: the protocol fixture recorder now accepts the mock
    server DNS address, records a sanitized DNS fixture as
    `protocol_family = "dns"`, and replays it through the same ready JSON
    address. DNS is now in the recorder corpus instead of being only a launcher
    smoke.
  - Proof: RED `uv run python -m pytest
    tests/test_protocol_fixture_recorder.py -q` failed on missing
    `dns_udp_addr`; GREEN `uv run python -m pytest
    tests/test_protocol_fixture_recorder.py tests/test_mock_server_launcher.py
    tests/test_release_doctor_contract.py -q`; `uv run ruff check
    scripts/protocol_fixture_recorder.py scripts/mock_server_runtime.py
    tests/test_protocol_fixture_recorder.py tests/test_mock_server_launcher.py`;
    `python3 -m py_compile scripts/protocol_fixture_recorder.py
    scripts/mock_server_runtime.py scripts/mock_server.py`.
  - 2026-06-13 progress: the same Python runtime now exposes
    `https_addr`/`https_base_url` and serves `/tiny` over a local TLS listener
    with the same request handler as HTTP. HTTPS fixture traffic is therefore
    in the shared protocol lab; Capsem MITM interception remains covered by the
    doctor/network routes that consume this lab.
  - Proof: RED `uv run python -m pytest
    tests/test_mock_server_launcher.py::test_mock_server_serves_https_fixture
    -q` failed on missing `https_base_url`; GREEN `uv run python -m pytest
    tests/test_mock_server_launcher.py::test_mock_server_serves_https_fixture
    tests/test_mock_server_launcher.py tests/test_protocol_fixture_recorder.py
    -q`; `uv run ruff check scripts/mock_server_runtime.py
    tests/test_mock_server_launcher.py tests/test_protocol_fixture_recorder.py`;
    `python3 -m py_compile scripts/mock_server_runtime.py
    tests/test_mock_server_launcher.py tests/test_protocol_fixture_recorder.py`.
  - 2026-06-13 correction: HTTPS mock traffic is a host-side fixture contract,
    while guest HTTPS remains the MITM rail. `local_fixture_env()` now carries
    `CAPSEM_MOCK_SERVER_HTTPS_BASE_URL` when ready JSON provides it, and
    `scripts/integration_test.py` propagates that value without inventing a
    second guest route.
  - Proof: RED `uv run python -m pytest
    tests/test_release_doctor_contract.py::test_mock_server_helper_exports_https_fixture_for_host_callers
    -q` failed before the helper exported the HTTPS fixture; GREEN same command
    (`1 passed`); `uv run python -m pytest tests/test_release_doctor_contract.py
    -q` (`18 passed`); `uv run ruff check scripts/mock_server.py
    scripts/integration_test.py tests/test_release_doctor_contract.py`;
    `python3 -m py_compile scripts/mock_server.py scripts/integration_test.py
    tests/test_release_doctor_contract.py`.
- [ ] RED/GREEN: every protocol lab case is a full-chain acceptance spec, not
  a status-code replay.
  - Suite home: `tests/ironbank/`.
  - Contract: `sprints/1.3-release-correction/IRONBANK.md`.
  - Authoring rule: use public route contracts, CLI docs/help, generated
    schemas, hermetic fixture definitions, observed client behavior, logs, DB
    rows, and route responses only. Do not read Rust/product internals to
    choose expected behavior.
  - Required assertion floor for each network/protocol test: at least ten
    explicit assertions covering (1) client-visible response, (2) parser
    family/type classification, (3) parsed request fields, (4) parsed response
    fields, (5) protocol-specific DB row, (6) unified security ledger row,
    (7) detection level/rule row when expected, (8) structured service/gateway
    log evidence, (9) in-memory status/stats counters, (10) UDS route output,
    (11) HTTP gateway route output, and (12) UI-facing JSON serialization
  - 2026-06-13 progress: `tests/ironbank/test_doctor_ledger.py` now extends
    the doctor ledger proof with MCP profile route contracts
    (`/profiles/{id}/mcp/default/info`, `/servers/list`, and
    `/servers/local/tools/list`), exact route field sets, built-in local tool
    names/descriptions/permission actions, MCP `tools/call` ledger byte and
    preview assertions, MCP builtin `net_events`, and the matching
    `mcp.tool_call` security-rule row. This closes the previous "MCP rows
    exist" weakness for the doctor stimulus, while the broader S4/S7 native
    MCP and streaming provider iron tests remain open.
  - Proof: RED
    `CAPSEM_TEST_PRESERVE_ALWAYS=1 uv run python -m pytest
    tests/ironbank/test_doctor_ledger.py::test_capsem_doctor_pays_protocol_and_security_ledger_debt
    -q -s --tb=short` first failed on incorrect MCP route assumptions, then
    GREEN passed (`1 passed in 31.67s`); `uv run ruff check
    tests/ironbank/test_doctor_ledger.py`; full suite
    `CAPSEM_TEST_PRESERVE_ALWAYS=1 uv run python -m pytest tests/ironbank/
    -q -s` (`3 passed in 37.53s`).
    shape when the route backs the UI.
  - Field-coverage invariant: each protocol spec must inspect every field it
    emits in all three public ledgers: structured log event, SQLite row(s), and
    UDS/HTTP route response. For each field, the test must either assert the
    exact value, assert a typed invariant/range/shape, or document it as
    not-applicable for that case. No uninspected DB/log/route field can be
    treated as covered. This includes nullable fields, defaults, timestamps,
    IDs, trace IDs, credential refs, rule IDs, detection levels, counters,
    byte counts, preview caps, body render metadata, status/decision/action
    enums, provider/model/tool names, paths, headers, protocol family/type,
    transport/IP/TCP/UDP facts, and error fields.
  - Schema drift guard: each full-chain spec must fail if the route response,
    DB table schema, or structured log schema gains a field that the field
    coverage ledger does not know about. New fields require new assertions or
    explicit not-applicable entries in the test fixture.
  - Required protocol specs:
    - HTTP must have at least twelve full-chain cases:
      1. accepted plain JSON request/response;
      2. denied request by CEL rule with client-visible denial body;
      3. asked request with ask ledger/status evidence;
      4. rewrite/preprocess request mutation with mutated upstream bytes and
         original/mutated audit rows;
      5. rewrite/postprocess response mutation with client-visible mutation;
      6. HTTPS/MITM JSON request/response with cert path and no fallback;
      7. gzip response decompression with parsed body and capped preview;
      8. chunked response with complete bytes/counters;
      9. SSE stream with event ordering, EOF, bytes, and no hyper error;
      10. WebSocket handshake/frame evidence;
      11. truncated upstream response with explicit error/partial ledger and
          route-visible diagnostic;
      12. large body/header preview capping with no raw credential leak.
    - DNS must have at least six full-chain cases:
      1. accepted A/AAAA query;
      2. accepted TXT query;
      3. denied domain by rule;
      4. malformed/truncated packet;
      5. long-label DNS-exfil detection;
      6. local/private answer with IP/TCP/UDP facts and default ask rule.
    - Model/OpenAI-compatible must have accepted, denied, truncated/error,
      non-stream JSON, SSE stream, tool declaration, executed tool call,
      tool response, token usage, thinking/reasoning, large prompt preview
      cap, and unknown-compatible-provider detection cases.
    - Model/Anthropic streaming must have accepted, denied, truncated/error,
      SSE text delta, tool_use/tool_result, usage delta, stop reason, EOF,
      response bytes, token counts, and no client-visible network error.
    - Model/Gemini-AGY streaming must have accepted, denied, truncated/error,
      Google internal endpoint classification, response text, thinking,
      tool deltas, token counts, OAuth/broker interaction, route/latest rows,
      and no client-visible network error.
    - MCP tools/list must prove server identity, resources/prompts/tools
      sections, no phantom executed calls, `mcp_calls`, security rows,
      route-visible server/tool evidence, UDS output, HTTP gateway output,
      counters, and UI serialization.
    - MCP tools/call must prove accepted, denied, ask, truncated/error,
      request args, response body, tool id/name, decision, `mcp_calls`,
      security rows, route/latest, counters, duplicate suppression, and
      separation from tools/list noise.
    - Credential broker/plugin must have at least five full-chain cases:
      1. OAuth auth-code/token response capture with `captured` verb;
      2. header/query/cookie API key capture with `captured` verb;
      3. stored-ref injection with `injected` verb and client success;
      4. brokered substitution/rewrite with `brokered` verb and no raw secret
         in DB/log/UI/debug;
      5. plugin disabled/ask/block/error modes with counters, detection level,
         structured logs, route status, and absolute block semantics.
    - File events must have accepted, denied, import, export, create, read,
      write/modify, delete, truncated/large content preview, symlink escape
      denial, path/name/ext/mime/content facts, DB rows, security rows, routes,
      counters, and logs.
    - Process events must have process audit observation, explicit exec,
      accepted exec, denied exec, failed exec, environment/argv preview caps,
      parent/child identity, DB rows, security rows, routes, counters, and
      logs.
    - Snapshot must be route-only and hermetic: route-created snapshot,
      compact created/modified/deleted summary, symlink escape denial, no
      snapshot rows in generic user activity unless explicitly requested, no
      DB hot-path read, route output, counters, and structured logs.
  - Current gap: existing recorder/replay tests prove fixtures are stable, but
    they do not yet prove Capsem's runtime parser/logger/security route
    contract.
  - 2026-06-12 progress: added the first Ironbank doctor ledger proof at
    `tests/ironbank/test_doctor_ledger.py`. It boots a VM through
    `/vms/create`, runs `capsem-doctor` against `capsem-mock-server`, and
    verifies `/history`, `/history/counts`, `/security/latest`, plus
    `net_events`, `dns_events`, `mcp_calls`, `model_calls`, `tool_calls`,
    `fs_events`, `exec_events`, `security_rule_events`, and
    `substitution_events` in the session DB. This caught and fixed model trace
    drift, missing non-streaming OpenAI-compatible tool-call ledger rows, and
    hermetic credential-broker storage/env propagation.
  - Proof: `cargo test -p capsem-core
    net::mitm_proxy::telemetry_hook::tests::openai_non_streaming_tool_call_carries_request_trace
    -- --nocapture`; `cargo test -p capsem-core
    net::ai_traffic::events::tests::non_streaming_openai_tool_calls --
    --nocapture`; `cargo test -p capsem-core
    credential_broker::tests::http_body_detector_finds_local_oauth_fixture_response
    -- --nocapture`; `cargo test -p capsem-core
    credential_broker::tests::http_body_credential_candidate_is_limited_to_known_exchange_paths
    -- --nocapture`; `cargo test -p capsem-service
    process_env_allowlist_forwards_mcp_timeout_knobs -- --nocapture`;
    `cargo build -p capsem-service -p capsem-process -p capsem-gateway -p
    capsem-mock-server`; `CAPSEM_TEST_PRESERVE_ALWAYS=1 uv run python -m
    pytest tests/ironbank/test_doctor_ledger.py -q -s` (`1 passed in
    34.55s`).
- [x] RED/GREEN: recorder creates sanitized fixtures with client/version,
  protocol family, auth mode, expected ledger rows, and expected visible bytes.
  - 2026-06-12 progress: `scripts/protocol_fixture_recorder.py` records
    schema-validated JSON fixtures from `capsem-mock-server` for
    Claude/Anthropic-shaped, Codex/OpenAI-compatible, AGY/Gemini-shaped,
    Ollama/OpenAI-compatible, OAuth token exchange, MCP tools/list,
    MCP tools/call, and credential-capture flows. Synthetic `capsem_test_*`
    secrets are recursively substituted as `credential:blake3:*` before
    writing.
  - Proof: `uv run python -m pytest tests/test_protocol_fixture_recorder.py
    -q` (`1 passed in 1.81s`); `uv run ruff check
    scripts/protocol_fixture_recorder.py tests/test_protocol_fixture_recorder.py`.
- [x] RED/GREEN: replay covers Claude/Anthropic, OpenAI/Codex-compatible,
  Gemini/AGY-compatible, Ollama/OpenAI-compatible, MCP, and credential flows.
  - 2026-06-12 progress: the recorder now exposes `replay_fixtures()`, which
    reissues recorded fixtures against the local lab and validates response
    status plus stable visible-byte counts. The test records and replays
    Claude/Anthropic-shaped, Codex/OpenAI-compatible, AGY/Gemini-shaped,
    Ollama/OpenAI-compatible, OAuth, MCP tools/list, MCP tools/call, and
    credential-capture fixtures without public network.
  - Proof: `uv run python -m pytest tests/test_protocol_fixture_recorder.py
    -q` (`2 passed in 0.92s`); `uv run ruff check
    scripts/protocol_fixture_recorder.py tests/test_protocol_fixture_recorder.py`.
- [x] RED/GREEN: live-local Ollama probe uses host `gemma4:latest` through the
  Capsem-routed path and records/replays the resulting native Ollama and
  OpenAI-compatible traffic without relying on an ad-hoc VM install.
  - 2026-06-12 proof: a fresh isolated `CAPSEM_HOME`/UDS service booted a
    named disposable session and reached host Ollama from inside the guest via
    `http://127.0.0.1:11434`, without installing Ollama in the guest. Native
    `/api/tags` returned `gemma4:latest`; OpenAI-compatible
    `/v1/chat/completions` returned model `gemma4:latest` and visible content
    `capsem`.
  - Ledger proof from that session DB:
    `net_events` contained `GET /api/tags` and
    `POST /v1/chat/completions` rows for `127.0.0.1:11434`, status `200`,
    decision `allowed`, and nonzero bytes. `model_calls` contained
    provider `ollama`, model `gemma4:latest`, method `POST`, path
    `/v1/chat/completions`, status `200`, and one parsed message. This proves
    the local backend path is routed and parsed through Capsem, not a guest
    install shortcut.
  - 2026-06-13 follow-up: Ironbank now asserts the exact security-rule ledger
    for the local OpenAI-compatible path: HTTP rows must include
    `profiles.rules.default_http`, `profiles.rules.ai_ollama_http_local_host`,
    and the `ask` guard from `profiles.rules.default_000_local_network`; model
    rows must include `profiles.rules.ai_openai_model_api` and
    `profiles.rules.default_model` with only allow actions.
  - Proof: `CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest
    tests/ironbank/test_model_sdk_ledger.py::test_openai_sdk_local_model_path_pays_full_ledger_debt_blackbox
    -q --tb=short`; `uv run ruff check
    tests/ironbank/test_model_sdk_ledger.py`.
  - Fresh proof after S4/S5 mock-server/DNS/doctor hardening:
    `CAPSEM_TEST_PRESERVE_ALWAYS=1 uv run python -m pytest
    tests/ironbank/test_model_sdk_ledger.py::test_openai_sdk_local_model_path_pays_full_ledger_debt_blackbox
    -q -s` (`1 passed in 2.97s`).
- [x] RED/GREEN: profile images ship Ollama through the builder/profile rail,
  not through manual VM repair.
  - 2026-06-12 progress: `config/profiles/{code,co-work}/build.sh` runs the
    official Ollama installer alongside Claude and AGY, `apt-packages.txt`
    includes `zstd`, and source `profile.toml` declares the `files.build`
    descriptor without generated pins.
  - Proof: `cargo test -p capsem-core profile_config -- --nocapture`; `cargo
    test -p capsem-admin profile_build -- --nocapture`; `cargo test -p
    capsem-admin image_workspace_materializes_self_contained_profile_config --
    --nocapture`; `uv run python -m pytest tests/test_docker.py -q -k
    'rootfs_keys or profile_root_and_build_script or config_input_record'`;
    `cargo run -p capsem-admin -- profile check config/profiles/code/profile.toml
    --config-root config --json`; `cargo run -p capsem-admin -- profile check
    config/profiles/co-work/profile.toml --config-root config --json`.
  - 2026-06-12 progress: profile build scripts now prune
    `/usr/local/lib/ollama/cuda_*` after the official install and both Code and
    Co-work Python requirement payloads include the `ollama` SDK. Source
    profile payload tests derive the paths from `profile.toml`, so this stays
    tied to the profile ledger rather than a hand-maintained file list.
  - Proof: `uv run python -m pytest
    tests/capsem-build-chain/test_profile_payload_contract.py -q`; `cargo run
    -p capsem-admin -- profile check config/profiles/code/profile.toml
    --config-root config --json`; `cargo run -p capsem-admin -- profile check
    config/profiles/co-work/profile.toml --config-root config --json`.
- [ ] RED/GREEN: Ironbank real-client Ollama proof covers OpenAI Python SDK,
  Anthropic/Claude SDK or CLI path, Codex, AGY, and LiteLLM where the client is
  scriptable without manual OAuth.
  - Required shape: each client routes through Capsem to host Ollama, writes a
    deterministic poem file in the guest, and proves model request/response,
    token counts, byte counts, tool-call/tool-response rows when applicable,
    file write rows, security/detection rows, UDS route output, HTTP route
    output, and session DB rows all agree.
  - Current debt: existing recorder/replay and live Ollama proof are useful,
    but they are still too thin; they do not yet prove real SDK/client
    behavior or file-writing agent outcomes.
  - 2026-06-12 progress: a black-box SDK presence probe against a fresh Code
    session showed `openai` and `anthropic` are missing from the current VM
    image while `httpx` and `requests` are present. The Code and Co-work
    profile package ledgers now include `openai`, `anthropic`, `litellm`, and
    `ollama` in source package files that capsem-admin validates and pins only
    during materialization. Remaining debt: rebuild EROFS assets from the
    profile rail, then add the real-client Ironbank test that exercises those
    SDKs through Capsem to host Ollama and validates DB/routes/logs.
  - Proof: `cargo run -p capsem-admin -- profile check
    config/profiles/code/profile.toml --config-root config --json`; `cargo run
    -p capsem-admin -- profile check config/profiles/co-work/profile.toml
    --config-root config --json`.
- [x] Proof: lab is shared by doctor, integration tests, recorder, and
  benchmark.
  - 2026-06-12 progress: renamed the canonical deterministic fixture service
    from `capsem-debug-upstream` to the shared mock server. The public contract
    is now `CAPSEM_MOCK_SERVER_BASE_URL`, with `scripts/mock_server.py` and
    `tests/helpers/mock_server.py` as the only launcher/helper path. This is
    the reusable mock boundary for doctor, integration, protocol recording,
    benchmark, and Ironbank; new feature-specific local servers are rejected.
  - 2026-06-12 progress: benchmark tests no longer carry a private fake HTTP
    fixture. `tests/test_capsem_bench_mitm_local.py` now starts the real
    shared mock server through the shared helper used by other
    hermetic tests, so HTTP/gzip/SSE/model/credential/WebSocket benchmark
    proof and doctor/integration proof cannot drift silently.
  - 2026-06-12 progress: release scripts no longer carry private
    mock-server process bootstrap code. `scripts/mock_server.py`
    is the single launcher/ready/lock/teardown helper, used by
    `scripts/doctor_session_test.py`, `scripts/integration_test.py`, the
    recorder tests, and benchmark tests.
  - 2026-06-12 correction: `capsem doctor` no longer links a Rust fixture
    crate. It spawns `scripts/mock_server_runtime.py`, reads the same ready
    JSON contract as Python tests, and fails loudly if the runtime is absent.
  - Proof: `uv run python -m pytest tests/test_release_doctor_contract.py -q`
    (`8 passed`); `uv run ruff check scripts/mock_server.py
    scripts/doctor_session_test.py scripts/integration_test.py
    tests/helpers/mock_server.py tests/test_release_doctor_contract.py`;
    `uv run python -m pytest tests/test_protocol_fixture_recorder.py
    tests/test_capsem_bench_mitm_local.py -q` (`25 passed`); `python3 -m
    py_compile scripts/mock_server.py scripts/doctor_session_test.py
    scripts/integration_test.py tests/helpers/mock_server.py`.

## S5. Doctor, Just, E2E, Benchmark

- [x] RED: `just smoke` fails if doctor is skipped or run in a reduced release
  mode.
  - 2026-06-11 progress: `capsem doctor --fast` is rejected by the CLI and
    `just smoke` invokes the full doctor command. The old reduced doctor rail
    is no longer an accepted release path.
  - Proof: `cargo test -p capsem parse_doctor -- --nocapture`; `uv run python
    -m pytest tests/test_release_doctor_contract.py -q`; `cargo check -p
    capsem`.
- [x] GREEN: remove release `--fast` escape and fold benchmark-only local
  server modes into standard `capsem-bench`.
  - 2026-06-11 progress: `mitm-local` is no longer a top-level
    `capsem-bench` mode. Local protocol scenarios run through
    `capsem-bench protocol` for release-scale numbers and through
    `capsem-bench all` when `CAPSEM_MOCK_SERVER_BASE_URL` points at the
    shared hermetic mock server for broad benchmark runs.
  - Proof: `uv run python -m pytest tests/test_capsem_bench_mitm_local.py
    -q`; `uv run python -m pytest
    tests/capsem-serial/test_mitm_local_benchmark.py -q`; `pnpm --dir docs
    build`.
- [ ] RED/GREEN: doctor exercises HTTP/HTTPS, gzip, chunked, SSE, WebSocket,
  DNS, MCP, model, OAuth/broker, file, process, import/export, local backend,
  snapshot route, blocked/error paths.
  - 2026-06-12 progress: in-VM doctor now posts a synthetic OAuth
    authorization-code token exchange to the local `capsem-mock-server`
    `/oauth/token` fixture. The test verifies HTTP 200 and response size while
    keeping synthetic `capsem_test_*` token values out of doctor output, so
    OAuth/broker stimulus is covered without real credentials or public
    providers.
  - Proof: `uv run python -m pytest tests/test_release_doctor_contract.py -q`
    (`10 passed`); `python3 -m py_compile
    guest/artifacts/diagnostics/test_network.py`; `(cd
    guest/artifacts/diagnostics && uv run python -m pytest --collect-only
    test_network.py -q)` (`39 tests collected`).
  - 2026-06-12 progress: strengthened Ironbank caught that the doctor model
    and OAuth stimuli were passing synthetic credentials in process argv. The
    network request path was brokered correctly, but `audit_events.argv` still
    preserved the raw test secret. Doctor now sends the same Authorization
    header and OAuth form through curl config/data files generated in the VM,
    so the MITM sees real credential-shaped traffic while process audit does
    not record the secret material.
  - 2026-06-12 progress: `/sbin/shutdown` is no longer a guest Capsem
    lifecycle alias. The TUI owns shutdown. Init removes any stale
    `/sbin/shutdown` alias, while `halt`, `poweroff`, `reboot`, and
    `/usr/local/bin/suspend` remain routed through `/run/capsem-sysutil`.
  - Proof: `python3 -m py_compile
    guest/artifacts/diagnostics/test_lifecycle.py
    guest/artifacts/diagnostics/test_network.py
    tests/test_release_doctor_contract.py
    tests/ironbank/test_doctor_ledger.py`; `uv run ruff check
    guest/artifacts/diagnostics/test_lifecycle.py
    guest/artifacts/diagnostics/test_network.py
    tests/test_release_doctor_contract.py
    tests/ironbank/test_doctor_ledger.py`; `uv run python -m pytest
    tests/test_release_doctor_contract.py::test_guest_init_publishes_rootfs_binaries_into_run_contract
    tests/test_release_doctor_contract.py::test_guest_network_doctor_exercises_oauth_fixture
    -q`. Full VM Ironbank rerun is intentionally held until the next asset
    swap; no rebuild was performed after the shutdown contract change.
  - 2026-06-13 progress: local HTTP/SSE/WebSocket/OAuth/model doctor fixtures
    no longer skip if `CAPSEM_MOCK_SERVER_BASE_URL` is missing or points at a
    port outside the guest redirect allowlist. That is a release wiring failure
    and now fails the diagnostic directly.
  - Proof: RED
    `uv run python -m pytest tests/test_release_doctor_contract.py::test_guest_network_doctor_requires_local_mock_server_instead_of_skipping -q`
    failed on `pytest.skip`; GREEN local network doctor contract subset passed:
    `uv run python -m pytest
    tests/test_release_doctor_contract.py::test_guest_network_doctor_requires_local_mock_server_instead_of_skipping
    tests/test_release_doctor_contract.py::test_guest_network_doctor_is_hermetic_by_default
    tests/test_release_doctor_contract.py::test_guest_network_doctor_exercises_oauth_fixture
    -q`. Additional proof: `uv run ruff check
    guest/artifacts/diagnostics/test_network.py
    tests/test_release_doctor_contract.py`; `python3 -m py_compile
    guest/artifacts/diagnostics/test_network.py
    tests/test_release_doctor_contract.py`.
  - 2026-06-13 progress: removed the last `pytest.skip` from the network
    doctor protocol proofs. The denied POST path now performs a real
    `curl -skX POST` to `evil-never-allowed.invalid` and requires either a
    transport failure or HTTP 403, so blocked/error coverage is no longer
    papered over by the default profile note.
  - Proof: RED `uv run python -m pytest
    tests/test_release_doctor_contract.py::test_guest_network_doctor_has_no_skipped_protocol_proofs
    -q` failed on the skipped POST proof; GREEN
    `uv run python -m pytest
    tests/test_release_doctor_contract.py::test_guest_network_doctor_has_no_skipped_protocol_proofs
    tests/test_release_doctor_contract.py::test_guest_network_doctor_exercises_oauth_fixture
    -q` (`2 passed`); full `uv run python -m pytest
    tests/test_release_doctor_contract.py -q` (`19 passed`); `uv run ruff
    check guest/artifacts/diagnostics/test_network.py
    tests/test_release_doctor_contract.py`; `python3 -m py_compile
    guest/artifacts/diagnostics/test_network.py tests/test_release_doctor_contract.py`.
  - Fresh VM proof after the denied POST change:
    `CAPSEM_TEST_PRESERVE_ALWAYS=1 uv run python -m pytest
    tests/ironbank/test_doctor_ledger.py::test_capsem_doctor_pays_protocol_and_security_ledger_debt
    -q -s --tb=short` (`1 passed in 31.61s`).
- [ ] RED/GREEN: doctor verifies DB ledger rows and rule/plugin evidence for
  allow/ask/block/disable/rewrite/pre/post/detection levels.
  - 2026-06-12 progress: `tests/ironbank/test_doctor_ledger.py` now proves the
    baseline doctor DB ledger for allow/default detection flow across HTTP,
    DNS, MCP, model/tool calls, file, exec, security-rule rows, and credential
    capture rows. Remaining debt: explicit ask/block/disable/rewrite/pre/post
    plugin and detection-level matrix.
  - Proof: `CAPSEM_TEST_PRESERVE_ALWAYS=1 uv run python -m pytest
    tests/ironbank/test_doctor_ledger.py -q -s` (`1 passed in 34.55s`).
  - 2026-06-12 progress: Ironbank now asserts the exact
    `/security/latest` JSON field set, closed rule action/detection-level
    vocabularies, exact `substitution_events` schema columns, broker outcome
    verbs, BLAKE3 reference shape, valid context JSON, and absence of raw
    synthetic secret markers across every text column in the session DB. The
    new checks found the argv leak above; after the doctor fixture source fix,
    the next rebuilt image must rerun this test before the gate closes.
  - Fresh proof after S4/S5 mock-server/DNS/doctor hardening:
    `CAPSEM_TEST_PRESERVE_ALWAYS=1 uv run python -m pytest
    tests/ironbank/test_doctor_ledger.py::test_capsem_doctor_pays_protocol_and_security_ledger_debt
    -q -s` (`1 passed in 31.35s`).
  - Combined Ironbank suite proof after the model, doctor, and package-manager
    refreshes: `CAPSEM_TEST_PRESERVE_ALWAYS=1 uv run python -m pytest
    tests/ironbank/ -q -s` (`3 passed in 37.39s`). Remaining S5/S7 debt is
    still explicit below: MCP-native iron tests, streaming provider replay,
    ask/block/disable/rewrite/pre/post matrix, and full `just test`.
- [x] RED/GREEN: doctor/toolchain probes cover apt/dpkg triggers, Python, pip,
  uv, Node, npm, npx, packaged CLIs, aliases, MCP bootstrap, DNS, TLS, FS
  writes.
  - 2026-06-12 progress: CA propagation is no longer implicit. Guest init now
    exports `SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`, and `NODE_EXTRA_CA_CERTS`
    for both the initial agent process and login shells, so TLS-sensitive
    doctor/toolchain probes inherit the Capsem CA consistently instead of
    depending on per-tool defaults.
  - Proof: `uv run python -m pytest tests/test_release_doctor_contract.py -q`;
    `sh -n guest/artifacts/capsem-init`.
  - 2026-06-12 progress: `_apt` sandbox failures were traced to guest `/`
    being `0700`, so non-root users could not traverse to `/bin/sh` despite
    the shell itself being executable. Guest init now normalizes the overlay
    root mode both before and inside the chroot after profile-root projection.
  - Runtime proof after `just run-service`: `target/debug/capsem run "stat -c
    '%a %U %G %n' /; su -s /bin/sh _apt -c 'id && touch
    /var/cache/apt/archives/partial/.capsem_apt_probe && rm -f
    /var/cache/apt/archives/partial/.capsem_apt_probe'"` returned `/` as
    `755 root root /` and `_apt` probe `OK`.
  - 2026-06-12 progress: pip, uv, npm, and apt doctor probes no longer hit
    public package registries. They generate local wheel/npm/deb fixtures
    inside the guest and install them through the real package managers.
    `capsem run "capsem-doctor -q -k 'term_is_xterm_256color or pip_install or
    uv_pip_install or uv_add_package or npm_install or apt_install or
    apt_partial_cache'"` passed `9 selected` tests in `1.53s` after repack.
    Previous public-registry doctor proof failed after `104.41s`, including two
    30s npm timeouts and uv retry delays, so this gate is now both hermetic and
    materially faster.
  - Proof: `uv run python -m pytest tests/test_release_doctor_contract.py -q`;
    `python3 -m py_compile guest/artifacts/diagnostics/test_runtimes.py`;
    selected in-VM doctor command above.
  - Full doctor proof after the hermetic fixes:
    `/usr/bin/time -p target/debug/capsem doctor` passed with `309 passed`,
    `13 skipped`, pytest time `23.72s`, wall time `26.20s`. The slowest tests
    are now snapshot/MCP filesystem checks (`2.28s` max), not network/package
    retries.
  - Stability/speed note for release reporting: before hermetic package
    fixtures, the comparable doctor run failed after `104.41s`, dominated by
    public registry retries and two 30s npm timeouts. After local wheel/npm/deb
    fixtures and CA propagation fixes, full doctor is passing in `26.20s` wall
    time, roughly a 4x improvement while removing public-network variance. The
    targeted package-manager probe is now `9 passed` in `1.53s`, so this gate
    can be run repeatedly while broadening coverage instead of burning minutes
    on registry instability.
  - 2026-06-13 progress: Winterfell/MCP fork and lifecycle fork benchmark
    package preservation now install a generated local `.deb` through the
    public VM file/exec routes and re-run the installed binary after fork.
    This keeps the test functional while removing public `apt` dependency from
    fork proof.
  - Proof: `uv run ruff check tests/helpers/package_probe.py
    tests/capsem-mcp/conftest.py tests/capsem-mcp/test_winter_is_coming.py
    tests/capsem-serial/test_lifecycle_benchmark.py`; `uv run python -m pytest
    tests/capsem-mcp/test_winter_is_coming.py -q --tb=short`;
    `CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest
    tests/capsem-serial/test_lifecycle_benchmark.py::test_fork_benchmark -q
    --tb=short`.
  - 2026-06-13 progress: Ironbank package-manager proof now includes `npx`
    against the same generated local npm package used by the npm proof, so
    no package-manager coverage depends on public registries or installed
    package theater.
  - Proof: RED `CAPSEM_TEST_PRESERVE_ALWAYS=1 uv run python -m pytest
    tests/ironbank/test_package_managers.py::test_package_managers_pay_their_ledger_debt_blackbox
    -q -s --tb=short` failed before the npx marker existed; GREEN same command
    (`1 passed in 3.19s`); `uv run ruff check
    tests/ironbank/test_package_managers.py`; full Ironbank
    `CAPSEM_TEST_PRESERVE_ALWAYS=1 uv run python -m pytest tests/ironbank/
    -q -s` (`3 passed in 37.95s`).
- [x] RED/GREEN: cargo test runner codesigning is serialized so parallel test
  shards do not race while replacing ad-hoc signatures.
  - 2026-06-11 progress: `scripts/run_signed.sh` now uses a portable
    `mkdir` lock around `codesign` and signature revalidation; no `flock`
    dependency on macOS.
  - Proof: `uv run python -m pytest
    tests/capsem-build-chain/test_run_signed.py -q`; `bash -n
    scripts/run_signed.sh`; parallel retry of `cargo test -p capsem-logger
    substitution_events_require_brokered_reference -- --nocapture` and `cargo
    test -p capsem-logger
    brokered_substitution_persists_reference_and_not_secret -- --nocapture`.
- [x] RED/GREEN: benchmarks use concurrency and request counts large enough to
  produce meaningful p50/p95/p99/rps for HTTP/SSE/WS/DNS/MCP/broker/model
  replay/storage/startup/lifecycle/fork.
  - 2026-06-13 progress: `just test` now keeps the Python non-serial
    integration suite under `pytest -n 4 --dist=loadfile` while running
    `tests/capsem-serial/` immediately afterward for timing and benchmark
    probes. This preserves the multi-VM canary and stops benchmark files from
    stealing the same Apple VZ launch budget from each other.
  - Proof: `CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest tests/ -v
    --tb=short -n 4 --dist=loadfile -m "not serial"
    --ignore=tests/capsem-recipes --ignore=tests/capsem-install
    --ignore=tests/capsem-build-chain` passed `1418 passed, 71 skipped` in
    `407.58s`.
  - Proof: `CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest
    tests/capsem-serial/ -v --tb=short -m serial` passed `11 passed, 1
    skipped` in `87.67s`, covering boot, exec latency, three-concurrent-VM
    latency, lifecycle/fork benchmarks, serial logs, and the baseline bench.
  - 2026-06-13 progress: the serial local MITM benchmark is no longer hidden
    behind `CAPSEM_RUN_MITM_LOCAL_BENCH=1` and no longer downshifts to
    `10` requests at concurrency `1`. The release contract now rejects that
    escape hatch, and the benchmark defaults run `50,000` requests at
    concurrency `64` through `capsem-mock-server`.
  - Proof: RED
    `uv run python -m pytest tests/test_release_doctor_contract.py::test_serial_benchmark_release_proofs_are_not_env_gated -q`
    failed on the env-gated skip; GREEN same command passed. Additional proof:
    `uv run ruff check tests/test_release_doctor_contract.py
    tests/capsem-serial/test_mitm_local_benchmark.py`; `uv run python -m
    pytest tests/test_capsem_bench_mitm_local.py -q` (`23 passed`).
  - 2026-06-13 progress: `capsem-bench protocol` is now a first-class
    benchmark mode for the local mock-server protocol suite, while the retired
    `capsem-bench mitm-local` escape hatch remains rejected. The serial VM
    release artifact defaults to high-sample model/credential scenarios instead
    of mixing 100+ GiB fixture transfer into the same 300s exec window.
  - Proof: RED
    `CAPSEM_REQUIRE_ARTIFACTS=1 CAPSEM_BENCH_TOTAL_REQUESTS=100
    CAPSEM_BENCH_CONCURRENCY=16 uv run python -m pytest
    tests/capsem-serial/test_mitm_local_benchmark.py::test_mitm_local_benchmark_artifact
    -q -s --tb=short` initially failed with `Unknown command: protocol` before
    `_pack-initrd` carried the new guest benchmark package into the boot asset.
    GREEN after `just _pack-initrd`: same low-count probe passed in `62.32s`.
    Release-scale GREEN after fixing a WebSocket close-timeout measurement bug:
    `CAPSEM_REQUIRE_ARTIFACTS=1 uv run python -m pytest
    tests/capsem-serial/test_mitm_local_benchmark.py::test_mitm_local_benchmark_artifact
    -q -s --tb=short` passed in `37.54s`, archived
    `benchmarks/mitm-local/data_1.3.1781205836_arm64.json`, and proved
    `model_json_response 50000/50000` at `3000.9 rps`, `18.8ms` p50,
    `58.0ms` p99 plus `credential_response 50000/50000` at `3029.0 rps`,
    `18.8ms` p50, `55.9ms` p99, both with zero errors and DB/no-secret checks.
    WebSocket echo now records `2508.2 fps`, `0.2ms` p50/p99 instead of
    spending the close timeout in the benchmark row.
- [x] RED/GREEN: failed suspend cannot leave a VM resumable from a partial
  Apple VZ checkpoint.
  - 2026-06-13 progress: `capsem-process` writes
    `checkpoint.vzsave.complete` only after save_state plus checkpoint/rootfs
    fsync succeeds. `capsem-service` treats a checkpoint as resumable only
    when both files exist, archives both on failed warm restore, and clears
    both after successful resume.
  - Proof: `cargo test -p capsem-service startup::tests -- --nocapture` (`8
    passed`); `cargo test -p capsem-service checkpoint -- --nocapture` (`3
    passed`); `cargo test -p capsem-process --no-run`; full `-n 4` Python
    canary above includes
    `tests/capsem-service/test_svc_suspend_corruption.py::TestSuspendOverlayDurability::test_suspend_failure_does_not_brick_vm`.

## S6. CEL and Security Event Contract

- [x] RED/GREEN: `ip`, `tcp`, and `udp` are first-party typed CEL facts.
  - 2026-06-11 progress: `SecurityEvent` now carries typed `ip`, `tcp`, and
    `udp` facts, exposes them through CEL, and serializes them through the
    public security-event DTO.
  - Proof: `cargo test -p capsem-core security_event_cel_ --lib --
    --nocapture`.
- [x] RED/GREEN: family and subobject `valid` booleans exist and are true CEL
  booleans.
  - 2026-06-11 progress: `valid` booleans exist for first-party roots and
    subobjects such as `model.request.valid`, `mcp.tool_call.valid`,
    `file.read.valid`, and `process.audit.valid`.
  - Proof: `cargo test -p capsem-core security_event_cel_ --lib --
    --nocapture`.
- [x] RED/GREEN: rule predicates cannot use `security.*`.
  - 2026-06-11 progress: `security.*` is no longer a first-party CEL root or
    `SecurityEvent` predicate surface; stale tests now match the original
    security event payload instead of rule-emitted decision state.
  - Proof: `cargo test -p capsem-core security_engine --lib -- --nocapture`.
- [x] RED/GREEN: default local/private/non-routable network rule is `ask`.
  - 2026-06-11 progress: built-in defaults now include
    `default.000_local_network`, an ordinary late default CEL rule whose action
    is `ask` for localhost/private/non-routable IP or host access.
  - Proof: `cargo test -p capsem-core security_rule_profile --lib --
    --nocapture`.
- [x] RED/GREEN: Ollama/local backend access changes only through explicit
  profile-owned rule actions: `allow`, `ask`, `block`, `disable`.
  - 2026-06-11 progress: profile-owned Ollama rules are proven for
    `allow`/`ask`/`block`; `disable` is represented by `enabled = false`, which
    keeps the rule in inventory and falls back to the default local ask guard.
  - Proof: `cargo test -p capsem-core security_rule_profile --lib --
    --nocapture`.
- [x] RED/GREEN: existing Ollama default/provider rules are audited so
  `localhost`, `127.0.0.1`, `host.docker.internal`, and `local.ollama` do not
  bypass the default local/private-network guard unless the profile's Ollama
  rule explicitly allows them.
  - 2026-06-11 progress: built-in Ollama local host access is an explicit
    `ai.ollama.rules.http_local_host` allow rule that wins before the default
    guard when enabled; the default guard still matches and remains visible for
    ledger evidence.
  - Proof: `cargo test -p capsem-core security_rule_profile --lib --
    --nocapture`.
- [x] RED/GREEN: all security ledger rows retain event id, trace id, rule id,
  action, detection level, plugin evidence, and event payload needed for
  forensics.
  - 2026-06-11 progress: runtime rule evaluation now records each matched
    rule's requested decision on the in-flight `SecurityEvent` before
    pre/postprocess plugins run, so later plugin/action ledger rows can be
    reconstructed against the rule decision that triggered them.
  - Proof: `cargo test -p capsem-core security_engine --lib -- --nocapture`.
  - 2026-06-11 proof refresh: `cargo check -p capsem-core`.

## S7. Runtime Protocol Fixes

- [ ] RED/GREEN: AGY/Gemini SSE produces client-visible bytes, parsed model
  rows, and no `hyper serve error`.
- [ ] RED/GREEN: Claude/Anthropic streaming produces client-visible bytes,
  parsed model rows, and no header/EOF corruption.
- [ ] RED/GREEN: tool declarations are not counted as executed tool calls.
- [ ] RED/GREEN: executed model tool calls and MCP tools/call rows are linked
  without phantom calls.
- [x] RED/GREEN: MCP user-facing stats distinguish executed tool calls from
  protocol chatter and host-only snapshot tooling.
  - 2026-06-11 progress: `DbReader::mcp_call_stats()` keeps filtering
    initialize/list/snapshot noise for UI/user status, while
    `raw_mcp_call_count()` exists for forensic session-index rollups that must
    equal raw `mcp_calls` ledger rows.
  - Proof: `cargo test -p capsem-logger mcp_call -- --nocapture`; `cargo
    check -p capsem-logger -p capsem-service`.
- [x] RED/GREEN: snapshot listing does not emit full per-file changes unless
  the MCP/CLI caller explicitly opts in.
  - 2026-06-11 progress: `snapshots_list` accepts `include_changes`, the
    guest `snapshots list --json --include-changes` flag forwards it, and
    doctor tests that require per-file change assertions opt in explicitly.
  - 2026-06-11 progress: the generated MCP tool catalog exposes
    `include_changes` on `snapshots_list`, so UI/TUI/tooling see the same
    explicit opt-in contract as the runtime handler.
  - Proof: `cargo test -p capsem-core list_snapshots --lib -- --nocapture`;
    `cargo test -p capsem-mcp-builtin
    snapshot_pagination_params_preserve_include_changes -- --nocapture`; `uv
    run python -m py_compile guest/artifacts/snapshots
    guest/artifacts/diagnostics/test_mcp.py`.
- [x] RED/GREEN: unknown AI-compatible protocol shape on unknown host emits
  model provider plus host and triggers detection.
  - 2026-06-13 closure: the hermetic mock server exposes `/model/shape`, a
    neutral non-provider path that returns an OpenAI-compatible response. The
    Ironbank SDK ledger proof posts an OpenAI-shaped JSON request there,
    verifies a `model_calls` row with `provider = openai`, validates the
    brokered credential ref, and proves `profiles.rules.ai_openai_model_api`
    plus `profiles.rules.default_model` fire from the security ledger.
  - Proof: `CAPSEM_TEST_PRESERVE_ALWAYS=1 uv run python -m pytest
    tests/ironbank/test_model_sdk_ledger.py::test_openai_sdk_local_model_path_pays_full_ledger_debt_blackbox
    -q -s --tb=short`; `cargo test -p capsem-core --lib
    provider_detection -- --nocapture`; `uv run ruff check
    tests/ironbank/test_model_sdk_ledger.py scripts/mock_server_runtime.py`.
- [ ] RED/GREEN: unknown remote MCP activity becomes route-visible profile
  evidence.
- [x] RED/GREEN: credential broker logs `captured`, `brokered`, `injected`, and
  errors without raw secret leakage or generic status fields.
  - 2026-06-11 progress: new `substitution_events` tables now CHECK broker
    outcomes against the closed verb set `captured|brokered|injected|error`;
    successful observed credential saves emit `captured`, stale `substituted`
    outcomes are rejected, and credential inventory exposes `injected_count`
    instead of stale substitution language.
  - 2026-06-13 closure: runtime capture now emits a second durable broker
    ledger row with outcome `brokered`; Ironbank verifies model SDK traffic
    produces `captured`, `brokered`, and `injected`, and body credentials emit
    both `captured` and `brokered` without raw secret leakage. The hermetic
    test credential store is locked so concurrent captures cannot corrupt or
    lose brokered refs before replay.
  - Proof: `cargo build -p capsem-process`; `CAPSEM_TEST_PRESERVE_ALWAYS=1 uv
    run python -m pytest
    tests/ironbank/test_model_sdk_ledger.py::test_openai_sdk_local_model_path_pays_full_ledger_debt_blackbox
    -q -s --tb=short`; `cargo test -p capsem-core --lib
    hook_writes_substitution_event_and_shared_credential_ref -- --nocapture`;
    `cargo test -p capsem-core --lib
    broker_test_store_preserves_concurrent_captures -- --nocapture`;
    `cargo test -p capsem-logger
    substitution_events_require_brokered_reference -- --nocapture`.

## S8. UI/TUI Contract Repair

- [x] RED/GREEN: user-facing dashboard says sessions/profiles, not VMs, except
  internal/debug contexts.
  - 2026-06-11 progress: dashboard headings, empty/loading states, create
    errors, lifecycle modals, toolbar menu/status, and stats subtitles now use
    session wording. The frontend build stamp is hidden on session tabs.
  - Proof: `pnpm --dir frontend test
    src/lib/__tests__/session-language-contract.test.ts`; `pnpm --dir
    frontend check`; targeted grep for retired visible VM labels is quiet.
- [x] RED/GREEN: profile cards render name, description, icon, readiness, asset
  checklist, `New`, and `Customize` from route data.
  - 2026-06-13 progress: dashboard profile cards no longer rely on a global
    customize-session button. Each profile card renders the route-provided
    name, description, icon, readiness text, and explicit actions: `New` for
    ready profiles, `Download` for missing assets, and `Customize` to open the
    create dialog preselected to that profile.
  - Proof: RED/GREEN `pnpm --dir frontend test
    src/lib/__tests__/session-language-contract.test.ts`; `pnpm --dir
    frontend test src/lib/__tests__/profile-page-contract.test.ts`; `pnpm
    --dir frontend check`.
  - 2026-06-13 progress: profile cards also render a compact `VM assets`
    checklist from `/profiles/{profile_id}/assets/status` with check,
    downloading, and missing indicators for the route-provided asset entries.
  - Proof: RED/GREEN `pnpm --dir frontend test
    src/lib/__tests__/session-language-contract.test.ts
    src/lib/__tests__/profile-page-contract.test.ts`; `pnpm --dir frontend
    check`.
- [x] RED/GREEN: incompatible/defunct sessions are greyed and expose only valid
  actions.
  - 2026-06-13 progress: the shared VM action helper now treats
    `Incompatible` and `Defunct` as terminal states and caps them to
    delete-only even if a stale `/status` payload includes `start`, `resume`,
    or `fork`. Dashboard rows already use this helper for clickability and
    action rendering.
  - Proof: RED/GREEN `pnpm --dir frontend test
    src/lib/__tests__/vm-actions.test.ts
    src/lib/__tests__/session-language-contract.test.ts`; `pnpm --dir
    frontend check`.
- [x] RED/GREEN: profile selection is route-backed and works with both `code`
  and `co-work`.
  - 2026-06-13 progress: Profile overview still uses the route-backed profile
    selector and broker inventory route, but no longer renders raw broker
    credential references. It shows provider, last-seen, observed, and
    injected counts in the primary UI.
  - Proof: `pnpm --dir frontend test
    src/lib/__tests__/profile-page-contract.test.ts
    src/lib/__tests__/stats-view-contract.test.ts`; `pnpm --dir frontend
    check`.
  - 2026-06-13 proof: route-backed selection and hyphenated profile IDs are
    covered by the service profile UI route matrix for both `code` and
    `co-work`, while the frontend profile page uses the selected profile id for
    info, credential broker, assets, enforcement, detection, plugin, and MCP
    sections.
  - Proof: `cargo test -p capsem-service
    profile_ui_route_matrix_is_registered_for_all_profiles -- --nocapture`;
    `pnpm --dir frontend test src/lib/__tests__/profile-page-contract.test.ts
    src/lib/__tests__/api.test.ts`.
- [x] RED/GREEN: enforcement/detection/plugins/MCP/assets pages load for both
  profiles with no 404/501.
  - 2026-06-13 progress: the frontend MCP page already called
    `/profiles/{profile_id}/mcp/default/info` and
    `/profiles/{profile_id}/mcp/default/edit`, and the service implemented
    both routes, but the gateway did not forward them. The gateway route
    matrix now covers both paths so profile MCP default policy controls cannot
    regress to a UI-visible 404.
  - Proof: RED/GREEN `cargo test -p capsem-gateway
    gateway_security_routes_are_explicitly_forwarded -- --nocapture`; `pnpm
    --dir frontend test src/lib/__tests__/api.test.ts`.
  - 2026-06-13 proof: the service route matrix now verifies profile info,
    assets, enforcement, detection, plugins, MCP, and skills routes for both
    `code` and `co-work`; the gateway explicit-forwarding matrix covers the
    profile route shapes forwarded to the service.
  - Proof: `cargo test -p capsem-service
    profile_ui_route_matrix_is_registered_for_all_profiles -- --nocapture`;
    `cargo test -p capsem-gateway
    gateway_security_routes_are_explicitly_forwarded -- --nocapture`; `pnpm
    --dir frontend test src/lib/__tests__/profile-page-contract.test.ts
    src/lib/__tests__/api.test.ts`.
- [x] RED/GREEN: plugin/MCP/rule modes use enum-backed selects/icons and
  disabled rows are visibly disabled.
  - 2026-06-13 progress: MCP default and per-tool permission selectors now
    render from a single typed `ToolPermission` option list instead of
    duplicated raw `<option>` values; plugin mode selectors already render from
    typed `PluginMode` metadata, and rule rows render typed action/detection
    metadata with disabled styling from the backend `enabled` field.
  - Proof: RED/GREEN `pnpm --dir frontend test
    src/lib/__tests__/mcp-section-contract.test.ts`; focused proof `pnpm
    --dir frontend test src/lib/__tests__/mcp-section-contract.test.ts
    src/lib/__tests__/plugin-section-contract.test.ts
    src/lib/__tests__/profile-page-contract.test.ts`; `pnpm --dir frontend
    check`.
- [x] RED/GREEN: stats detail panels show one canonical presentation and move
  raw JSON to debug-only.
  - 2026-06-11 progress: stats detail drawers no longer render the selected
    row once as full raw JSON and again as repeated fields. Scalar fields are
    shown once; payload/header/body fields render as dedicated highlighted
    sections.
  - Proof: `pnpm --dir frontend test
    src/lib/__tests__/stats-view-contract.test.ts`; `pnpm --dir frontend
    check`.
- [x] RED/GREEN: HTTP/DNS/file/process/security/credentials panels use correct
  labels, counts, syntax highlighting, and no duplicate payload fields.
  - 2026-06-11 progress: file stats cards now summarize the visible
    created/modified/deleted ledger actions instead of unrelated
    import/export/brokered-ref counters.
  - 2026-06-11 progress: credential broker stats now render broker evidence
    as captured/brokered/injected event verbs, hide BLAKE3 credential
    references from the primary table/detail presentation, and remove the old
    status/reference table columns. Backend verb/schema normalization remains
    tracked in S7.
  - 2026-06-11 progress: security stats now show complete action and detection
    summaries, including zero-count enum values, instead of elevating a partial
    blocks/rules-hit headline.
  - 2026-06-13 progress: process stats now separate command execution rows
    from observed process inventory, replace the unrelated process credential
    reference card with a unique-binary count, show observed argv/command
    context, and remove visible tutorial prose from the app.
  - 2026-06-13 progress: stats detail payload sections now choose syntax
    highlighting by field/value shape: HTTP headers use the HTTP grammar,
    JSON previews parse/format as JSON, and non-JSON payloads stay escaped
    text instead of a fake JSON panel.
  - 2026-06-13 progress: credential broker metric cards now count captured,
    brokered, injected, and error rows independently; total broker events stays
    a separate total and unknown outcomes render as error instead of silently
    becoming captured.
  - Proof: `pnpm --dir frontend test
    src/lib/__tests__/stats-view-contract.test.ts`; `pnpm --dir frontend
    check`.

## S9. Agent Bootstrap Repair

- [x] RED/GREEN: profile root contains non-secret AGY config/wrapper and does
  not contain OAuth token/log/conversation/history/cache files.
  - 2026-06-13 progress: Profile payload contracts now require AGY non-secret
    settings, the AGY build wrapper that preserves `agy-real` and adds
    `--dangerously-skip-permissions`, Claude bootstrap state, Codex MCP config,
    and the shared root MCP config. The test rejects checked-in root payload
    paths containing OAuth/token/log/conversation/history/cache material.
  - Proof: `uv run python -m pytest
    tests/capsem-build-chain/test_profile_payload_contract.py -q`; `cargo test
    -p capsem-admin`.
- [x] RED/GREEN: Claude install/bootstrap includes MCP approval and dangerous
  mode acknowledgement without first-run prompts.
  - 2026-06-12 progress: Code and Co-work profile roots now package
    `/root/.claude/settings.local.json` with `enabledMcpjsonServers =
    ["capsem"]`, matching the live accepted Claude evidence from preserved
    sessions, and both `root.manifest.json` files pin the non-secret approval
    payload. The profile payload contract fails if a profile declares the
    built-in `capsem` MCP server without the Claude approval file.
  - Proof: `uv run python -m pytest
    tests/capsem-build-chain/test_profile_payload_contract.py -q`; `cargo run
    -p capsem-admin -- profile check config/profiles/code/profile.toml
    --config-root config --json`; `cargo run -p capsem-admin -- profile check
    config/profiles/co-work/profile.toml --config-root config --json`.
- [x] RED/GREEN: Claude binary/install path is valid or doctor reports exact
  remediation; no broken symlink in shipped profile.
  - 2026-06-13 progress: The profile build hook contract asserts Claude is
    installed through the profile build rail and promoted to
    `/usr/local/bin/claude` instead of relying on a broken home-directory
    symlink.
  - Proof: `uv run python -m pytest
    tests/capsem-build-chain/test_profile_payload_contract.py -q`.
- [x] RED/GREEN: Codex config/MCP/bootstrap files are profile-owned and pinned.
  - 2026-06-13 progress: `root/.codex/config.toml` must declare the `capsem`
    MCP server command `/run/capsem-mcp-server`, and the root manifest must pin
    that file exactly.
  - Proof: `uv run python -m pytest
    tests/capsem-build-chain/test_profile_payload_contract.py -q`.
- [x] RED/GREEN: profile root manifest hashes every shipped bootstrap file.
  - 2026-06-13 progress: `capsem-admin profile check` now walks the profile
    `root/` seed directory and rejects any unlisted regular file before image
    materialization can copy it. It also rejects duplicate manifest entries,
    stale/missing entries, and non-regular root payloads.
  - Proof: RED `cargo test -p capsem-admin
    profile_check_rejects_unpinned_profile_root_payload_files -- --nocapture`
    failed before the admin fix; GREEN `cargo test -p capsem-admin`; `cargo run
    -p capsem-admin -- profile check config/profiles/code/profile.toml
    --config-root config --json`; `cargo run -p capsem-admin -- profile check
    config/profiles/co-work/profile.toml --config-root config --json`.
- [ ] Proof: fresh VM can start AGY/Claude/Codex bootstrap paths without
  mutating unpinned profile state before first model request.

## S10. Packaging, Install, Docs, Release Gate

- [x] RED/GREEN: `.pkg` and `.deb` fail if they contain rootfs/initrd/kernel
  asset blobs.
  - 2026-06-11 progress: package builders now stage only the selected
    manifest, manifest provenance, binaries, and profile ledger. VM asset
    blobs remain external and are reconciled by the service from the installed
    manifest.
  - Proof: `uv run python -m pytest
    tests/capsem-build-chain/test_install_asset_payload.py
    tests/test_repack_deb.py tests/test_build_pkg.py -q`; `bash -n
    scripts/build-pkg.sh scripts/repack-deb.sh scripts/deb-postinst.sh
    scripts/pkg-scripts/postinstall`.
- [ ] GREEN: package accepts local/remote manifest override, copies it to the
  service-owned location, and records origin/hash in status/debug/install log.
- [x] GREEN: package postinstall hydrates local manifest assets without
  embedding VM blobs in the package.
  - Root cause from full `just test`: the `.deb` installed
    `manifest.json`/profiles but never materialized
    `$CAPSEM_HOME/assets/{arch}/{hash-name}`, so installed-layout validation
    failed on missing `vmlinuz-<hash16>`.
  - Fix: `capsem update --assets` now reads local package
    `manifest-origin.json`, copies from the source asset tree through
    `copy_missing_local_assets`, verifies blake3, and writes the same
    hash-named layout as remote downloads. `.pkg` and `.deb` postinstall call
    that public reconciler and fail with `asset_hydration_failed` if it fails.
  - Proof: `cargo test -p capsem-core copy_missing_local_assets --
    --nocapture`; `cargo test -p capsem local_manifest_asset_source --
    --nocapture`; `uv run python -m pytest
    tests/capsem-build-chain/test_install_asset_payload.py
    tests/capsem-install/test_installed_layout.py::TestInstalledLayoutContract::test_hash_named_assets_exist
    -q`; `just test-install` passes 39/39 install checks with 22 skips and
    logs `event=assets_hydrated`.
- [x] GREEN: install logs are timestamped and actionable for manifest/profile
  copy, asset hydration success/failure, service registration, and completion.
- [x] Proof: `just test-install` builds a CI-like package and installs through
  the package path.
- [x] RED/GREEN: bootstrap frontend dependency installation is non-interactive
  in the full gate.
  - Root cause: `bootstrap.sh -y` still ran bare `pnpm install
    --frozen-lockfile`, and pnpm aborted in non-TTY mode when it needed to
    recreate `frontend/node_modules`.
  - Fix: both bootstrap pnpm install branches run with `CI=true`.
  - Proof: `uv run python -m pytest
    tests/capsem-bootstrap/test_dev_setup.py::TestDevSetup::test_bootstrap_pnpm_install_is_noninteractive
    -q`; `sh bootstrap.sh -y` passes with doctor 37 passed / 1 skipped.
- [x] RED/GREEN: fork-of-fork must not boot from a malformed copied
  `session.db`.
  - Root cause from full `just test`: `capsem_fork` cloned only the main
    `session.db` file. A live VM may have committed rows in `session.db-wal`,
    so a fork created from a fork could boot with a malformed or incomplete DB
    image.
  - Fix: `clone_sandbox_state` now snapshots `session.db` through SQLite
    `VACUUM INTO`, then opens the clone and runs `quick_check`. The clone is a
    standalone DB and does not copy WAL/SHM sidecars.
  - Proof: `cargo test -p capsem-core clone_sandbox_state -- --nocapture`;
    `uv run python -m pytest
    tests/capsem-mcp/test_fork_images.py::test_fork_of_fork -q`.
- [x] RED/GREEN: profile-dependent code must survive arbitrary profile ids
  before returning to the shipping `code`/`co-work` names.
  - Trap: checked-in `config/profiles/code` and `config/profiles/co-work`
    were temporarily renamed to `mary` and `jane` and every live expectation
    was updated to those ids.
  - Proof: full `just test` passed under the temporary profile ids, including
    Ironbank, integration, benchmark, Linux package build, and install E2E.
  - Restoration proof: profiles were renamed back to `code` and `co-work`;
    `just _materialize-config`; `cargo test -p capsem-core profile_contract
    -- --nocapture`; `cargo test -p capsem-admin -- --nocapture`; and
    `uv run python -m pytest tests/test_build_assets_profile.py
    tests/capsem-build-chain/test_source_profiles_unpinned.py
    tests/test_injection_script.py tests/test_integration_script_profiles.py
    -q` all passed with the shipping ids.
  - 2026-06-13 follow-up: `scripts/injection_test.py` now defaults to
    `target/config/profiles`, accepts `--profiles-dir`, forwards
    `CAPSEM_PROFILES_DIR`, and uses a short `/tmp` CAPSEM_HOME so injection
    scenarios exercise the same materialized profile catalog as packages/CI
    without hitting macOS UDS path limits.
  - Proof: `uv run ruff check scripts/injection_test.py
    tests/test_injection_script.py`; `uv run python -m pytest
    tests/test_injection_script.py -q --tb=short`.
  - 2026-06-13 follow-up: `doctor --fix` build-assets repair now loops over
    `config/profiles/*/profile.toml` and invokes `just build-assets
    <profile_id> <arch>` for every checked-in profile instead of rebuilding a
    default-only asset set.
  - Proof: `bash -n scripts/doctor-common.sh`; `uv run python -m pytest
    tests/test_release_doctor_contract.py -q --tb=short` (`15 passed`).
- [ ] Proof: status/debug show service version, manifest origin/hash, profile
  status, plugin status, route status, doctor evidence, OBOM/SBOM references.
  - 2026-06-13 progress: support-bundle tests now expect the current
    `config/settings.toml` path, gateway mock fixtures include route-provided
    VM `available_actions`, MITM gateway tests use the test fixture corp config
    path, and the release cleanup Rust formatting debt is cleared.
  - Proof: `cargo fmt --check`; `cargo test -p capsem-core
    security_event_log_sanitizer_logging_plugin_redacts_before_logger_emit --
    --nocapture`; `cargo test -p capsem support_bundle -- --nocapture`;
    `cargo test -p capsem redact -- --nocapture`; `uv run python -m pytest
    tests/capsem-gateway/test_mitm_policy.py -q --tb=short`; and `uv run ruff
    check tests/capsem-gateway/conftest.py
    tests/capsem-gateway/test_mitm_policy.py`.
- [ ] Proof: changelog, docs, skills, and benchmark docs updated.
- [ ] Proof: full final gates pass and branch is pushed.

## Coverage Ledger

- Unit/contract: Pending. Must cover profile schema, route contracts, CEL
  objects, package payloads, plugin contracts, event ledgers.
- Functional: Pending. Must cover admin materialization, service routes, UI/TUI
  calls, doctor probes, broker flows.
- Adversarial: Pending. Must cover malformed configs, stale hashes, invalid
  rule roots, raw secret leak attempts, symlink escapes, bad streams, ENOSPC.
- E2E/VM/integration: Pending. Must cover fresh package install, fresh sessions,
  hermetic protocol lab, doctor, real-session DB proof.
- Telemetry/observability: Pending. Must cover structured gateway logs, plugin
  counters, security ledger, install logs, debug/status payloads.
- Performance: Pending. Must cover CEL, plugin latency, DB writer, HTTP/SSE/WS,
  DNS, MCP, broker, model replay, startup, lifecycle, fork, storage.

## Notes

- Manual evidence sessions must not be destroyed without user approval.
- S1 proof so far: `uv run python -m pytest
  tests/capsem-build-chain/test_no_legacy_user_config.py -q`; `cargo test -p
  capsem-core --lib policy_config -- --nocapture`; `cargo test -p
  capsem-core credential_broker -- --nocapture`; `cargo check -p capsem-core
  -p capsem-service -p capsem-process -p capsem-mcp-builtin`.
- S1 2026-06-11 focused burn proof: `uv run python -m pytest
  tests/capsem-install/test_setup_removed.py
  tests/capsem-service/test_svc_settings.py
  tests/capsem-build-chain/test_no_legacy_user_config.py -q`; `cargo check
  -p capsem-core -p capsem-service`.
- S1 2026-06-11 reload/e2e sweep proof: live code/test grep for
  `load_settings_files`, `user.toml`, `CAPSEM_USER_CONFIG`,
  `save_mcp_user_config`, `load_mcp_user_config`, and `user_config_path` is
  quiet outside the guard test; `uv run python -m pytest
  tests/capsem-build-chain/test_no_legacy_user_config.py -q`; `cargo check -p
  capsem-process`; `uv run python -m py_compile
  tests/capsem-e2e/test_framed_mcp_mitm.py`.
- S1 correction from review: any VM/profile behavior that survived as local
  settings is still debt. `settings.toml` is not a new name for `user.toml`;
  behavior must move to profile-owned artifacts or be rejected.
- S1 burn detail: runtime loaders now validate local settings and corp files
  with owner-specific contracts; credential broker no longer writes provider
  discovery into settings; no-argument profile batch-update/config-discovery
  helper symbols were removed. A broad `cargo test -p capsem-core policy_config`
  invocation tried to run the signed `mitm_integration` wrapper and failed at
  local codesign; the equivalent `--lib` policy-config proof is green.
- S1 admin/materialization proof: `cargo test -p capsem-admin -- --nocapture`
  passes after adding a failing/green check for malformed profile-owned MCP
  JSON and requiring generated image workspaces to pass `profile check` rather
  than parse-only validation.
- S1 package proof: `cargo test -p capsem-admin
  profile_check_rejects_empty_profile_package_file_even_when_hash_matches --
  --nocapture` passes; the full capsem-admin suite is now 29/29 green.
- S1 config-root proof: `cargo test -p capsem-admin -- --nocapture` passes
  31/31; `cargo test -p capsem-core --lib policy_config::ownership --
  --nocapture`, `cargo test -p capsem-core --lib policy_config::corp_provision
  -- --nocapture`, and `cargo test -p capsem-core --lib policy_config::loader
  -- --nocapture` pass after moving authored corp TOML to
  `refresh_policy = "24h"` while keeping internal `CorpSource`
  refresh-interval metadata numeric.
- S1 service proof: `cargo build -p capsem-service && uv run python -m pytest
  tests/capsem-service/test_svc_install.py -q` passes 16/16. The Python
  service fixture initially failed before rebuilding `target/debug/capsem-service`,
  confirming this route was testing the runnable service binary.
- `code-mq9ymjb2` shows apt/mandb permission and guest ENOSPC evidence.
- `code-mq9x5edq` shows AGY OAuth token reached guest disk; broker must own it.
- `code-mq9ye61s` shows Claude install/bootstrap and streaming failures.
- Host Ollama local baseline checked on 2026-06-11:
  `127.0.0.1:11434/api/tags` reports `gemma4:latest` with completion, tools,
  and thinking capabilities. This is the preferred local backend for hermetic
  model/protocol debugging, routed through Capsem.
- The current `sprints/1.3-debug-loop/current-hotlist.md` remains source
  evidence, but new implementation status belongs here.
