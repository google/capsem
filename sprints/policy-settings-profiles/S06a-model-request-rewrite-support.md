# S06a - Model Request Rewrite Support

## Goal

Implement `model.request` rewrite support so rewrite rules can transform outbound
model request bodies instead of failing closed as "not implemented yet".

This sprint is dedicated and explicit so the engine contract matches the v1 rule
format promised in S04.

## Scope

- Support `model.request` rewrite for `request.data` in v1.
  (Spec earlier used "request.body" as prose shorthand; the
  canonical field name in the v1 condition / rewrite_target
  vocabulary is `request.data`, matching the existing
  `request.data.contains("...")` syntax in `if` clauses and the
  vocabulary of other rewrite targets such as `response.text`.)
- Remove the current unsupported-rewrite deny path for this callback.
- Preserve fail-closed behavior for invalid rewrite config, invalid regex, and
  rewrite execution failures.
- Keep existing `model.response`, `model.tool_call`, and
  `model.tool_response` rewrite behavior unchanged.
- Ensure matched `policy.model.<rule_name>` action/reason still appear in
  telemetry/debug/status outputs.

## Tasks

- [ ] Implement `model.request` rewrite path in
      `policy_model::evaluate_model_request_policy`.
- [ ] Support rewrite target `request.body` for this callback in v1.
- [ ] Keep deterministic deny behavior for invalid rewrite or rewrite runtime
      failures (with explicit policy reason).
- [ ] Add unit tests for successful rewrite, no-match pass-through, invalid
      rewrite fail-closed, and malformed/truncated body handling.
- [ ] Add integration tests in MITM path proving rewritten body is forwarded
      upstream and telemetry shows rewrite action/rule.
- [ ] Confirm debug/status surfaces continue showing matched
      `policy.model.<rule_name>` for rewrite decisions.

## Verification Gate

Run after implementation:

```sh
cargo test -p capsem-core policy_model
cargo test -p capsem-core policy_model_request_rewrite
cargo test -p capsem-core -p capsem-service -p capsem-process
```

## Coverage Ledger

- Unit/contract:
  - rewrite target parsing for `request.body`
  - capture/ref validation behavior
  - deterministic rule ordering and matched-rule reporting
- Functional:
  - `model.request` rewrite mutates request body and continues upstream
  - no-match path is unchanged
- Adversarial:
  - invalid regex / invalid rewrite target / invalid rewrite value fail closed
  - malformed/truncated JSON body handling is deterministic and non-leaky
- E2E/VM:
  - model request rewrite behavior visible in real MITM request flow
- Telemetry:
  - rewrite decisions keep policy action/rule/reason visibility
- Performance:
  - rewrite path only pays body-processing cost when model request rewrite rules
    are active
