# Profile V2 Runtime Hygiene

## Goal

Close the next structural gap after S06: keep Profile V2 runtime policy,
HTTP decompression, CEL matching, and builder-generated settings aligned
before moving on to later sprints.

## Scope

- Add a dedicated `net::policy_v2` import surface for runtime policy types.
- Keep legacy settings/defaults APIs under `net::policy_config`.
- Audit gzip handling through the MITM response path and add regression tests
  for model response policy over compressed upstream bodies.
- Add focused CEL tests for HTTP response fields and header access.
- Add builder tests proving image build context and generated defaults come
  from the same guest config.

## Decisions

- Start with a facade rename rather than moving every type out of
  `policy_config`. That gives call sites a clean dependency boundary while
  avoiding churn in the large settings loader module.
- Treat decompression as two related paths:
  - streaming guest/telemetry decompression in `DecompressionHook`
  - full-body model policy decoding in `policy_v2_model`
- Prefer tests that prove the production boundary rather than only private
  helpers.

## Files

- `crates/capsem-core/src/net/policy_v2.rs`
- `crates/capsem-core/src/net/mod.rs`
- Policy V2 runtime call sites in `capsem-core`, `capsem-process`, and
  `capsem-service`
- `crates/capsem-core/src/net/policy_config/tests.rs`
- `crates/capsem-core/src/net/mitm_proxy/tests.rs`
- `tests/test_docker.py`
- `CHANGELOG.md`

## Done

- Runtime policy call sites import `Policy*` types from `net::policy_v2`.
- CEL tests cover response-body, response-header, and mixed request/response
  expressions.
- MITM tests prove gzip-compressed model responses are evaluated before guest
  delivery and do not leak blocked text.
- Builder tests prove enabled AI CLI package installs and generated defaults
  are derived from the same config object.
- Focused Rust/Python tests, format checks, and compile checks pass or any
  remaining debt is explicit in the tracker.

## Testing Proof Matrix

- Unit/contract: Policy V2 facade compile tests, CEL expression tests,
  builder config/defaults alignment tests.
- Functional: MITM model response gzip integration test through the proxy
  response path.
- Adversarial: compressed blocked response text must not leak to the guest;
  invalid/missing header fields remain false in CEL.
- E2E/VM: not in this sprint unless the focused gates uncover a runtime gap.
- Telemetry: MITM model response denial test asserts policy/session fields.
- Performance: no benchmark planned; scope is structural hygiene and coverage.
