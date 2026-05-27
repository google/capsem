# Service and Process Findings

Status: completed; transferred to T7 FD05 and owner rows in T3/T5/T8/T10.
Downstream implementation remains open.

Agent: Kierkegaard (`019e1264-dcba-79b0-9159-bebebceea23a`)
T8 hook scope audit agent: Gibbs (`019e1342-9f35-7261-a62f-953938ceb395`)
T8 reload/telemetry audit agent: Mendel (`019e1342-9fe8-7b81-b5cb-39d3712ef196`)

## Scope

- Policy reload/apply semantics.
- Service routes and auth coverage.
- Cleanup behavior.
- Environment forwarding.
- Helper discovery.
- Runtime config integration.

## Findings

- [x] [P0] MCP helper install/discovery can fail silently.
  - Paths: `crates/capsem-process/src/main.rs:725`,
    `scripts/repack-deb.sh:34`, `scripts/deb-postinst.sh:34`,
    `scripts/simulate-install.sh:42`,
    `tests/capsem-install/conftest.py:89`.
  - Detail: `capsem-process` searches for `capsem-mcp-aggregator` beside
    itself, then falls back to an empty stub if missing; builtin helper is also
    skipped if absent. Linux install scripts/tests still encode old six-binary
    layout.
  - Proof: installed layout tests require `capsem-mcp-aggregator` and
    `capsem-mcp-builtin`.
  - Sprint IDs: T5.1, T10.1.
  - Transfer status: resolved in T5; generated package inspection remains
    T10/T11.

- [x] [P1] Settings save/apply semantics are not release-safe.
  - Paths: `crates/capsem-service/src/main.rs:2700`,
    `crates/capsem-service/src/main.rs:2650`,
    `crates/capsem-process/src/ipc.rs:502`,
    `crates/capsem-core/src/net/policy_config/loader.rs:160`,
    `tests/capsem-service/test_svc_core.py:50`,
    `tests/capsem-service/test_svc_settings.py:47`.
  - Detail: `POST /settings` persists and returns refreshed tree but does not
    apply to running sessions; reload result has no persisted/applied state or
    failed IDs; process reload can warn/default but still Pong.
  - Sprint IDs: T5.5, T8.4, T10.5.
  - Transfer status: resolved for T5 targeted path; T8 added structured
    reload failure JSON, frontend saved-but-not-applied state, and a live
    `/settings` + `/reload-config` Policy V2 E2E path. T10 still owns the
    focused VM proof run.

- [x] [P1] Running builtin MCP HTTP/domain policy is startup-only.
  - Paths: `crates/capsem-process/src/main.rs:334`,
    `crates/capsem-process/src/ipc.rs:508`,
    `crates/capsem-mcp-builtin/src/main.rs:456`,
    `tests/capsem-e2e/test_framed_mcp_mitm.py:820`.
  - Detail: builtin server reads domain policy env once at startup; reload
    updates in-process locks but cannot update already-spawned builtin env.
  - Sprint IDs: T5.5, T8.4, T10.5.
  - Transfer status: resolved for T5 refresh/reload path; T8 updates the
    framed MCP E2E to warm the builtin HTTP server before settings reload, then
    prove refreshed domain policy. T10 still owns the focused VM proof run.

- [x] [P1] `McpRefreshTools` drops builtin wiring and masks errors.
  - Paths: `crates/capsem-process/src/ipc.rs:619`,
    `crates/capsem-service/src/main.rs:3004`,
    `tests/capsem-service/test_svc_mcp_api.py:82`.
  - Detail: refresh uses plain `build_server_list`, losing builtin
    server/session env; service ignores returned `McpRefreshResult` and always
    reports success.
  - Sprint IDs: T5.5, T8.4, T10.5.
  - Transfer status: resolved in T5.

- [x] [P1] Helper child env is not explicitly isolated.
  - Paths: `crates/capsem-process/src/main.rs:800`,
    `crates/capsem-core/src/mcp/server_manager.rs:324`.
  - Detail: aggregator and external stdio MCP children spawn without
    `env_clear()`, inheriting process env plus configured `def.env`.
  - Proof: adversarial env-leak test for external stdio servers.
  - Sprint IDs: T5.3, T10.5.
  - Transfer status: resolved in T5.

- [ ] [P1] Cleanup is still partly non-deterministic.
  - Paths: `crates/capsem-service/src/main.rs:3843`,
    `crates/capsem-service/src/main.rs:3787`,
    `crates/capsem-service/src/main.rs:4081`,
    `crates/capsem-service/src/main.rs:4827`,
    `tests/capsem-session-lifecycle/test_wal_cleanup.py:31`.
  - Detail: delete/stop/purge return while cleanup runs fire-and-forget;
    shutdown removes ephemeral session dirs before waiting for process cleanup.
  - Sprint IDs: T5.3, T10.5.

- [x] [P1] Hook dispatch is not integrated through service/process.
  - Paths: `crates/capsem-service/src/main.rs:2987`.
  - Detail: scoped crates show only spec route/test, no `PolicyHookClient`,
    endpoint config, IPC payload, or runtime dispatch. Hook dispatch cannot
    ship unless T8 wires it elsewhere.
  - Sprint IDs: T8.1, T8.2, T8.3, T3.
  - T8 audit: Gibbs reconfirmed the service route surface is Spec0-only and no
    process/runtime endpoint propagation exists.
  - Transfer status: deferred for `1.1.xxx`; release UI/settings/docs reject
    or describe configured external hook dispatch as infrastructure-only.

- [x] [P2] `/policy-hook/spec` lacks route/auth coverage.
  - Paths: `crates/capsem-service/src/tests.rs:1283`,
    `crates/capsem-service/src/main.rs:4605`.
  - Proof: add service/gateway route matrix test for `/policy-hook/spec`.
  - Sprint IDs: T5.2, T10.5.
  - Transfer status: resolved in T5.

## Tests Not Run

- Static code-reading investigation only; no tests were run.

## T5.3/T5.5 Execution Audit, 2026-05-10

Agent: Volta (`019e1312-61f4-7622-b6e6-ebc4fc63b508`)

Status: completed; findings captured for T5.3 and T5.5.

### Findings

- [x] [P1] `capsem-process` still spawns the MCP aggregator with inherited env.
  - Release impact: helper processes can inherit Capsem config override paths,
    test-only env, API tokens, or other parent env not intended for MCP
    runtime.
  - Paths: `crates/capsem-process/src/main.rs:800`.
  - Required proof: aggregator spawn calls `env_clear()` and only adds explicit
    safe trace/runtime vars.
  - Sprint IDs: T5.3, T10.5.
  - Transfer status: resolved in T5.

- [x] [P1] External stdio MCP children still inherit aggregator env.
  - Release impact: third-party stdio MCP servers can receive parent process
    environment by default instead of only configured `def.env`, trace vars,
    and Capsem helper vars.
  - Paths: `crates/capsem-core/src/mcp/server_manager.rs:324`.
  - Required proof: env-leak fixture proves sentinel Capsem config/test/API env
    vars are absent while configured vars and trace vars remain.
  - Sprint IDs: T5.3, T10.5.
  - Transfer status: resolved in T5.

- [x] [P2] Trace propagation to stdio children is accidental.
  - Release impact: fixing broad env inheritance without explicit trace
    forwarding would break child trace correlation.
  - Paths: `crates/capsem-core/src/mcp/server_manager.rs:326`.
  - Required proof: manager forwards explicit trace env after `env_clear()`.
  - Sprint IDs: T5.3, T6, T10.5.
  - Transfer status: resolved in T5; T6 timeline display fixture is resolved,
    with real-session trace proof remaining T8/T10.

- [x] [P2] One clean-exit cleanup path still blocks a Tokio task.
  - Release impact: expected child exit can perform recursive filesystem
    deletion directly on an async worker.
  - Paths: `crates/capsem-service/src/main.rs:606`,
    `crates/capsem-service/src/main.rs:4599`.
  - Required proof: expected-exit ephemeral cleanup and periodic stale cleanup
    run through `spawn_blocking` or an equivalent async-safe cleanup path.
  - Sprint IDs: T5.3, T10.5.
  - Transfer status: resolved in T5.

- [x] [P1] `/settings` persists but does not apply or report apply state.
  - Release impact: the UI can report saved settings while running sessions
    continue with stale policy/domain/MCP state.
  - Paths: `crates/capsem-service/src/main.rs:2766`,
    `frontend/src/lib/stores/settings.svelte.ts:197`.
  - Required proof: service API returns persisted/applied/failed-session state
    or the frontend preserves saved-but-not-applied details from reload.
  - Sprint IDs: T5.5, T8.4, T10.5.
  - Transfer status: resolved for T5 targeted path; full running-session E2E
    remains T8/T10.

- [x] [P1] `McpRefreshTools` still uses the non-builtin builder and masks
  refresh failures.
  - Release impact: live MCP refresh can drop local builtin tools, keep stale
    domain policy env, and still return success to the service/frontend.
  - Paths: `crates/capsem-process/src/ipc.rs:621`,
    `crates/capsem-mcp-aggregator/src/main.rs:360`,
    `crates/capsem-service/src/main.rs:3070`.
  - Required proof: refresh uses builtin-aware server construction with
    regenerated session/db/domain/trace env, and service aggregates refresh
    failures instead of discarding them.
  - Sprint IDs: T5.5, T8.4, T10.5.
  - Transfer status: resolved for T5; running-VM E2E remains T8/T10.

### Tests Not Run

- Static code-reading investigation only; no builds/tests were run.
