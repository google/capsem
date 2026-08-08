# Sprint: 1.3 Main Cleanup

## Tasks

- [x] Create sprint artifacts.
- [x] Audit changelog claims against implementation.
- [x] Align EROFS defaults to lz4hc level 12.
- [x] Remove setup wizard/setup authority references from current docs/defaults/UI text.
- [x] Add plugin policy examples to default templates.
- [x] Expose plugin policy in the UI with mode/detection-level selects.
- [x] Burn old runtime security enforcement paths.
- [x] Update or delete stale policy tests.
- [x] Update docs and changelog to match code.
- [x] Update release-process skill benchmark gate.
- [x] Update docs benchmark results page with current 1.3 numbers and zstd/lz4hc note.
- [x] Run focused tests.
- [ ] Run/update benchmark artifacts.
- [ ] Run `just smoke`.
- [ ] Run `just test`.
- [ ] Commit clean milestones.

## Changelog Audit

- [x] Kernel 7.0 claim verified.
- [x] EROFS claim adjusted to lz4hc-12 default plus optional zstd.
- [x] Install/setup claim verified and stale setup references removed.
- [x] Security-event rule spine claim verified after T2.
- [ ] Plugin endpoint/default claim verified after T1.
- [x] PySigma claim verified.
- [x] DB writer/security ledger claim verified.
- [ ] Observability/benchmark claims verified.

## Notes

- Discovery: runtime still contains old Policy V2/NetworkPolicy/MCP decision rails.
- Discovery: `SecurityEvent.detections` already supports multiple rule/plugin detection records.
- Decision: approved EROFS rootfs default is `lz4hc` level `12`; zstd remains optional support only because macOS and Linux benchmark evidence did not justify zstd for the speed-first 1.3 target.
- Audit: see `changelog-audit.md`; EROFS docs/defaults, setup references, old runtime rails, and benchmark docs remain red.
- Test: `uv run pytest tests/test_models.py tests/test_config.py tests/test_docker.py tests/test_settings_spec.py -q` passed with 364 tests.
- T2 burn: deleted old MITM PolicyHook/Policy V2 HTTP/model files, old framed-MCP decision provider shapes, stale DNS/MCP/MITM tests tied to removed rails, and the MCP built-in legacy domain bridge.
- T2 routing: HTTP request, model request/response, framed MCP request/response, MCP built-in HTTP tools, and DNS query blocking now evaluate canonical `SecurityEvent` through `SecurityRuleSet`/CEL plus plugin policy before materialization/dispatch.
- T2 caveat: `NetworkPolicy` remains in runtime only for non-enforcement mechanics still outside the security-event rule contract: HTTP body/port settings and DNS redirect/cache coherence.
- T2/T3 compatibility burn: deleted the retired callback policy config/TS surface, old domain/http policy modules, and settings response `policy` payload; `[policy.*]` TOML and save keys are now explicit rejection tests.
- T2/T3 validation: `cargo test -p capsem-core --no-default-features --lib` passed with 1642 tests and 1 ignored; MITM integration passed 26 tests with 1 ignored throughput test; `cargo test -p capsem-service --no-default-features` passed 90 lib + 106 bin tests; frontend `pnpm check && pnpm test` passed with 352 tests; `cargo check -p capsem-process --no-default-features` and `cargo check -p capsem-mcp-builtin --no-default-features` passed.

## Coverage Ledger

- Unit/contract:
  - `cargo test -p capsem-core --no-default-features builtin_http_security -- --nocapture` passed (8 tests).
  - `cargo test -p capsem-core --no-default-features fetch_http_blocked_domain -- --nocapture` passed (2 tests).
  - `cargo test -p capsem-core --no-default-features dns_handler_blocks_query_through_security_event_rules -- --nocapture` passed.
  - `cargo test -p capsem-core --no-default-features --no-run` passed.
  - `cargo test -p capsem-core --no-default-features --lib` passed (1642 passed, 1 ignored).
  - `cargo test -p capsem-service --no-default-features` passed (90 lib + 106 bin tests).
  - `cargo check -p capsem-process --no-default-features` passed.
  - `cargo check -p capsem-mcp-builtin --no-default-features` passed.
- Frontend/UI:
  - `pnpm check && pnpm test` passed from `frontend/` (352 Vitest tests).
  - Browser verification still pending.
- Functional:
  - Service settings save rejects retired `policy.*` keys atomically.
  - MITM integration passed (26 passed, 1 ignored throughput test).
- Adversarial:
  - Built-in HTTP invalid URL/scheme tests fail before network.
  - Built-in HTTP and DNS block tests prove CEL rules stop materialization/upstream dispatch.
- E2E/VM:
  - Pending smoke and targeted VM paths.
- Telemetry:
  - DNS and built-in HTTP denied rows carry `security_event` policy mode/action/rule/reason fields.
  - Full session DB endpoint verification still pending smoke/VM gates.
- Performance:
  - Pending fresh benchmark artifacts and docs benchmark update.
- Missing/deferred:
  - Linux-only KVM/filesystem failures may need Monday Linux-team run.
