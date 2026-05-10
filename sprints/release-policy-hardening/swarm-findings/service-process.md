# Service and Process Findings

Status: completed, pending transfer into T3/T5/T8/T10.

Agent: Kierkegaard (`019e1264-dcba-79b0-9159-bebebceea23a`)

## Scope

- Policy reload/apply semantics.
- Service routes and auth coverage.
- Cleanup behavior.
- Environment forwarding.
- Helper discovery.
- Runtime config integration.

## Findings

- [ ] [P0] MCP helper install/discovery can fail silently.
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

- [ ] [P1] Settings save/apply semantics are not release-safe.
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

- [ ] [P1] Running builtin MCP HTTP/domain policy is startup-only.
  - Paths: `crates/capsem-process/src/main.rs:334`,
    `crates/capsem-process/src/ipc.rs:508`,
    `crates/capsem-mcp-builtin/src/main.rs:456`,
    `tests/capsem-e2e/test_framed_mcp_mitm.py:820`.
  - Detail: builtin server reads domain policy env once at startup; reload
    updates in-process locks but cannot update already-spawned builtin env.
  - Sprint IDs: T5.5, T8.4, T10.5.

- [ ] [P1] `McpRefreshTools` drops builtin wiring and masks errors.
  - Paths: `crates/capsem-process/src/ipc.rs:619`,
    `crates/capsem-service/src/main.rs:3004`,
    `tests/capsem-service/test_svc_mcp_api.py:82`.
  - Detail: refresh uses plain `build_server_list`, losing builtin
    server/session env; service ignores returned `McpRefreshResult` and always
    reports success.
  - Sprint IDs: T5.5, T8.4, T10.5.

- [ ] [P1] Helper child env is not explicitly isolated.
  - Paths: `crates/capsem-process/src/main.rs:800`,
    `crates/capsem-core/src/mcp/server_manager.rs:324`.
  - Detail: aggregator and external stdio MCP children spawn without
    `env_clear()`, inheriting process env plus configured `def.env`.
  - Proof: adversarial env-leak test for external stdio servers.
  - Sprint IDs: T5.3, T10.5.

- [ ] [P1] Cleanup is still partly non-deterministic.
  - Paths: `crates/capsem-service/src/main.rs:3843`,
    `crates/capsem-service/src/main.rs:3787`,
    `crates/capsem-service/src/main.rs:4081`,
    `crates/capsem-service/src/main.rs:4827`,
    `tests/capsem-session-lifecycle/test_wal_cleanup.py:31`.
  - Detail: delete/stop/purge return while cleanup runs fire-and-forget;
    shutdown removes ephemeral session dirs before waiting for process cleanup.
  - Sprint IDs: T5.3, T10.5.

- [ ] [P1] Hook dispatch is not integrated through service/process.
  - Paths: `crates/capsem-service/src/main.rs:2987`.
  - Detail: scoped crates show only spec route/test, no `PolicyHookClient`,
    endpoint config, IPC payload, or runtime dispatch. Hook dispatch cannot
    ship unless T8 wires it elsewhere.
  - Sprint IDs: T8.1, T8.2, T8.3, T3.

- [ ] [P2] `/policy-hook/spec` lacks route/auth coverage.
  - Paths: `crates/capsem-service/src/tests.rs:1283`,
    `crates/capsem-service/src/main.rs:4605`.
  - Proof: add service/gateway route matrix test for `/policy-hook/spec`.
  - Sprint IDs: T5.2, T10.5.

## Tests Not Run

- Static code-reading investigation only; no tests were run.
