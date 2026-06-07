# Profile V2 HTTP/CEL/Builder Hardening

## Goal

Stress the areas that can quietly break Profile V2 in production: HTTP body decompression before policy evaluation, CEL condition semantics, and the Python builder path that generates VM images plus settings artifacts.

## Scope

- Audit HTTP decompression in MITM/model response policy paths.
- Add focused CEL tests for edge semantics that Profile V2 rules depend on.
- Inspect the Python capsem-builder config/defaults/image generation path and add guard tests where assumptions are thin.
- Keep changes test-first where possible and focused to hardening.

## Decisions

- Do not reintroduce legacy settings/defaults loading.
- Prefer small contract tests around existing code over broad rewrites unless an actual bug is found.
- Treat malformed compressed bodies as fail-closed only where the policy boundary is about to make a body-dependent decision.

## Files To Inspect

- `crates/capsem-core/src/net/mitm_proxy/*`
- `crates/capsem-core/src/net/policy_v2/*`
- `src/capsem/builder/config.py`
- `src/capsem/builder/docker.py`
- `src/capsem/builder/models.py`
- `tests/test_config.py`
- `tests/test_docker.py`

## Done

- Decompression assumptions are documented in tracker and covered by focused tests.
- CEL edge cases have direct Policy V2 tests.
- Builder/settings generation assumptions are covered by Python tests or explicitly documented as no-change findings.
- Focused Rust/Python verification passes.

## Testing Proof Matrix

- Unit/contract: Policy V2 CEL/decompression tests; builder generation tests.
- Functional: MITM policy hook/model paths through their production-facing helper boundaries.
- Adversarial: malformed or mismatched compression; CEL absent fields/type mismatches.
- E2E/VM: deferred unless code changes imply a VM behavior risk.
- Telemetry: preserve existing policy decision field assertions.
- Performance: no benchmark unless decompression code changes hot-path behavior.
