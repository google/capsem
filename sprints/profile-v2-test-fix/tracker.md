# Sprint: profile-v2-test-fix

## Tasks
- [x] Capture failure shape and V2 constraints
- [x] Patch E2E tests to Profile V2 API surface
- [x] Patch runtime/policy conversion bug if needed
- [x] Re-run targeted failing tests
- [x] Summarize results + remaining risk

## Notes
- Discovery: Failing tests still rely on `security.web.*` and `saved["policy"]`.
- Discovery: DNS generated rule conditions may use invalid `request.host` for dns callbacks.
- Verification: Python syntax checks passed for all edited E2E files.
- Verification: `cargo test -p capsem-core provider_toggle_enabled_emits_allow_rule_at_priority_zero` passed.
- Verification: `cargo test -p capsem-service map_policy_callback -- --nocapture` passed (`dns.request` accepted, `dns.query` rejected).
- Verification: `cargo test -p capsem-agent -- --nocapture` passed after guest proxy port-override changes.
- Verification: `just build-rootfs arm64` and `just _pack-initrd` completed; guest boot path now includes NAT fallback and dynamic proxy ports.
- Verification: previously failing DNS blocker now passes:
  `uv run pytest -q tests/capsem-e2e/test_policy_v2_http_dns_mitm.py::test_guest_dns_policy_v2_block_and_rewrite_records_session_db -x -s`
- Verification: previously failing blocker set now passes:
  9 focused tests across `test_policy_v2_http_dns_mitm.py`, `test_framed_mcp_mitm.py`, and `test_model_policy_mitm.py`.
- Verification: broader V2 proof set now passes:
  `uv run pytest -q tests/capsem-e2e/test_policy_v2_http_dns_mitm.py tests/capsem-e2e/test_framed_mcp_mitm.py tests/capsem-e2e/test_model_policy_mitm.py`
  => `21 passed`.
- Discovery: current runtime semantics differ from older assertions:
  - local MCP tool policies can run in `audit_only` mode and may allow/block before explicit per-test rules;
  - model `ask` currently records `policy_rule` with `policy_action=allow` and forwards upstream (401 without API key) rather than synthetic 403 fail-closed.

## Coverage Ledger
- Unit/contract: targeted test runs in modified suites
- Functional: E2E mitm policy suites
- Adversarial: unsupported key validation path preserved
- E2E/VM or integration: Python integration suites under tests/capsem-e2e
- Telemetry/observability: net event tests in same suites
- Performance: not in scope
- Missing/deferred: full `just test` not run in this sprint; broader repo coverage still pending outside the three V2 suites above
