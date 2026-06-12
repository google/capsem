# Sprint: 1.3 Release Correction

## Current Rule

No new AGY/Claude/Codex/OAuth manual run until the local due-diligence gates
below pass. Manual credentials are not the debugger.

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

## S1. Profile/Config Authority

- [x] RED: test that any read/write/use of `user.toml`, `CAPSEM_USER_CONFIG`,
  `user_config_path`, or `load_settings_files` fails the contract.
- [x] GREEN: remove the legacy user config rail from service/runtime/broker/MCP
  tests/benchmarks/helpers.
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

- [ ] RED: integration tests fail if protocol paths hit public services.
- [ ] GREEN: one local protocol lab serves HTTP, HTTPS/MITM, DNS, SSE,
  WebSocket, MCP JSON-RPC, OAuth/OIDC, and model fixture replay.
- [ ] RED/GREEN: recorder creates sanitized fixtures with client/version,
  protocol family, auth mode, expected ledger rows, and expected visible bytes.
- [ ] RED/GREEN: replay covers Claude/Anthropic, OpenAI/Codex-compatible,
  Gemini/AGY-compatible, Ollama/OpenAI-compatible, MCP, and credential flows.
- [ ] RED/GREEN: live-local Ollama probe uses host `gemma4:latest` through the
  Capsem-routed path and records/replays the resulting native Ollama and
  OpenAI-compatible traffic without installing Ollama in the guest.
- [ ] Proof: lab is shared by doctor, integration tests, recorder, and
  benchmark.

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
    `capsem-bench` mode. Local MITM scenarios run only through
    `capsem-bench all` when `CAPSEM_BENCH_MITM_LOCAL_BASE_URL` points at the
    shared hermetic debug upstream.
  - Proof: `uv run python -m pytest tests/test_capsem_bench_mitm_local.py
    -q`; `uv run python -m pytest
    tests/capsem-serial/test_mitm_local_benchmark.py -q`; `pnpm --dir docs
    build`.
- [ ] RED/GREEN: doctor exercises HTTP/HTTPS, gzip, chunked, SSE, WebSocket,
  DNS, MCP, model, OAuth/broker, file, process, import/export, local backend,
  snapshot route, blocked/error paths.
- [ ] RED/GREEN: doctor verifies DB ledger rows and rule/plugin evidence for
  allow/ask/block/disable/rewrite/pre/post/detection levels.
- [ ] RED/GREEN: doctor/toolchain probes cover apt/dpkg triggers, Python, pip,
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
- [ ] RED/GREEN: benchmarks use concurrency and request counts large enough to
  produce meaningful p50/p95/p99/rps for HTTP/SSE/WS/DNS/MCP/broker/model
  replay/storage/startup/lifecycle/fork.

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
- [ ] RED/GREEN: unknown AI-compatible protocol shape on unknown host emits
  model provider plus host and triggers detection.
- [ ] RED/GREEN: unknown remote MCP activity becomes route-visible profile
  evidence.
- [ ] RED/GREEN: credential broker logs `captured`, `brokered`, `injected`, and
  errors without raw secret leakage or generic status fields.
  - 2026-06-11 progress: new `substitution_events` tables now CHECK broker
    outcomes against the closed verb set `captured|brokered|injected|error`;
    successful observed credential saves emit `captured`, stale `substituted`
    outcomes are rejected, and credential inventory exposes `injected_count`
    instead of stale substitution language.
  - Proof: `cargo test -p capsem-logger
    substitution_events_require_brokered_reference -- --nocapture`; `cargo
    test -p capsem-logger --lib
    brokered_substitution_persists_reference_and_not_secret -- --nocapture`;
    `cargo test -p capsem-core --lib
    hook_writes_substitution_event_and_shared_credential_ref -- --nocapture`;
    `cargo test -p capsem-service
    credential_broker_plugin_runtime_reports_session_db_captures --
    --nocapture`; `pnpm --dir frontend test
    src/lib/__tests__/stats-view-contract.test.ts src/lib/__tests__/api.test.ts`;
    `cargo check -p capsem-core -p capsem-logger -p capsem-service`; `pnpm
    --dir frontend check`.

## S8. UI/TUI Contract Repair

- [x] RED/GREEN: user-facing dashboard says sessions/profiles, not VMs, except
  internal/debug contexts.
  - 2026-06-11 progress: dashboard headings, empty/loading states, create
    errors, lifecycle modals, toolbar menu/status, and stats subtitles now use
    session wording. The frontend build stamp is hidden on session tabs.
  - Proof: `pnpm --dir frontend test
    src/lib/__tests__/session-language-contract.test.ts`; `pnpm --dir
    frontend check`; targeted grep for retired visible VM labels is quiet.
- [ ] RED/GREEN: profile cards render name, description, icon, readiness, asset
  checklist, `New`, and `Customize` from route data.
- [ ] RED/GREEN: incompatible/defunct sessions are greyed and expose only valid
  actions.
- [ ] RED/GREEN: profile selection is route-backed and works with both `code`
  and `co-work`.
- [ ] RED/GREEN: enforcement/detection/plugins/MCP/assets pages load for both
  profiles with no 404/501.
- [ ] RED/GREEN: plugin/MCP/rule modes use enum-backed selects/icons and
  disabled rows are visibly disabled.
- [x] RED/GREEN: stats detail panels show one canonical presentation and move
  raw JSON to debug-only.
  - 2026-06-11 progress: stats detail drawers no longer render the selected
    row once as full raw JSON and again as repeated fields. Scalar fields are
    shown once; payload/header/body fields render as dedicated highlighted
    sections.
  - Proof: `pnpm --dir frontend test
    src/lib/__tests__/stats-view-contract.test.ts`; `pnpm --dir frontend
    check`.
- [ ] RED/GREEN: HTTP/DNS/file/process/security/credentials panels use correct
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
  - Proof: `pnpm --dir frontend test
    src/lib/__tests__/stats-view-contract.test.ts`; `pnpm --dir frontend
    check`.

## S9. Agent Bootstrap Repair

- [ ] RED/GREEN: profile root contains non-secret AGY config/wrapper and does
  not contain OAuth token/log/conversation/history/cache files.
- [ ] RED/GREEN: Claude install/bootstrap includes MCP approval and dangerous
  mode acknowledgement without first-run prompts.
- [ ] RED/GREEN: Claude binary/install path is valid or doctor reports exact
  remediation; no broken symlink in shipped profile.
- [ ] RED/GREEN: Codex config/MCP/bootstrap files are profile-owned and pinned.
- [ ] RED/GREEN: profile root manifest hashes every shipped bootstrap file.
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
- [ ] GREEN: install logs are timestamped and actionable.
- [ ] Proof: `just install` builds CI-like package and installs through package
  path.
- [ ] Proof: status/debug show service version, manifest origin/hash, profile
  status, plugin status, route status, doctor evidence, OBOM/SBOM references.
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
