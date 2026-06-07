# Sprint: Profile V2 Runtime Hygiene

## Tasks

- [x] Create sprint plan and tracker.
- [x] Add `net::policy_v2` runtime facade.
- [x] Rename runtime policy imports away from `net::policy_config`.
- [x] Add CEL tests for HTTP response/body/header assumptions.
- [x] Add gzip model-response policy regression test.
- [x] Add builder config/defaults alignment tests.
- [x] Update changelog.
- [x] Run focused Rust and Python verification.
- [x] Commit functional milestone.

## Notes

- Discovery: `policy_config` still mixes legacy settings/defaults APIs with
  Policy V2 runtime types. The first hygienic step is a stable facade, not a
  wholesale file move.
- Discovery: HTTP streaming decompression and model-response policy decoding
  are separate paths. Both need coverage because one protects guest delivery
  and the other protects pre-delivery policy decisions.
- Red/green: `policy_v2_runtime_call_sites_use_policy_v2_import_surface`
  failed on `policy_confirm.rs` before the facade rename, then passed after
  runtime imports moved to `net::policy_v2`.
- Discovery: compressed model responses already decode before Policy V2
  response matching; the new gzip regression locks that down.
- Verification: format, whitespace, focused Policy V2 Rust tests, builder
  alignment tests, compile checks, and capsem-process runtime tests passed.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-core policy_v2_ --lib -- --nocapture`;
  `cargo test -p capsem-process mcp_runtime -- --nocapture`;
  `uv run --group dev python -m pytest tests/test_docker.py::TestGuestConfigBuildAndDefaultsAlignment -q`.
- Functional: MITM gzip model-response block test exercises the proxy response
  path through upstream dispatch and guest-facing denial.
- Adversarial: compressed blocked model text and missing HTTP response headers
  must not leak or match accidentally.
- E2E/VM: deferred unless focused tests uncover a boundary issue.
- Telemetry: gzip model-response denial asserts net-event policy action/rule
  and redacted response preview.
- Performance: not in scope; no hot-loop changes planned.
- Missing/deferred: physical extraction of Policy V2 types out of
  `policy_config/types.rs` may become its own sprint if the facade exposes
  more coupling than expected. Full `just test` and VM smoke are still needed
  before calling the wider Profile V2 train release-ready.
