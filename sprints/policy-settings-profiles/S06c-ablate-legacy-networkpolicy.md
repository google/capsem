# S06c - Ablate Legacy NetworkPolicy Runtime

## Status

Done.

## Goal

Remove the legacy `NetworkPolicy` runtime and the V1 MITM policy hook so Profile
V2 policy is the only active runtime authority for DNS/HTTP/model/MCP network
decisions.

This closes the V1 runtime that survived the earlier settings/defaults removal.
The follow-up rename milestone can then collapse `policy` names to `policy`
without carrying two policy concepts.

## Scope

- Delete `crates/capsem-core/src/net/policy.rs`.
- Delete `crates/capsem-core/src/net/mitm_proxy/policy_hook.rs` and tests.
- Remove the V1 hook from production MITM pipeline registration.
- Remove the legacy `policy: SharedPolicy` field from MITM, DNS, VM registry,
  and process runtime state.
- Collapse DNS shared policy to the `PolicyConfig` handle that is currently
  named `SharedPolicy`.
- Reroute DNS full-block behavior to `dns.request` Policy rules.
- Remove hardcoded `NetworkPolicy::http_upstream_ports`; plain HTTP policy is
  governed by Profile V2 rules and the network engine plan.
- Preserve body preview capture behavior with explicit MITM constants until the
  later settings schema exposes the knobs.
- Update runtime/tests so migrated V1 denial behavior is proved by equivalent
  V2 rules.

## Non-Goals

- Do not do the broad `policy` naming collapse here. That belongs to the
  post-S06 cleanup milestone after the legacy runtime is gone.
- Do not remove `policy_hook_events` session tables yet. S08b decides which
  tables become projections and how the resolved-event journal replaces them.
- Do not redesign enforcement semantics. S08a/S08b own real CEL, detection, and
  Security Engine architecture.

## Testing Matrix

- Unit/contract: source guard proves `net::policy`, `NetworkPolicy`, and
  MITM `policy_hook` are absent from active runtime code.
- Functional: DNS block/rewrite and HTTP allow/block behavior are driven by
  `PolicyConfig` only.
- Adversarial: invalid Policy conditions fail closed; policy reload swaps
  only the V2 config and cannot leave a stale V1 policy behind.
- Integration: MITM pipeline registers Policy HTTP hook and no V1 policy
  hook; capsem-process boot/reload shares the same Policy handle across DNS,
  HTTP, MCP, and model paths.
- Regression: migrated legacy domain-block cases are represented as V2 rules
  and still block.

## Done Means

- `crates/capsem-core/src/net/policy.rs` and
  `crates/capsem-core/src/net/mitm_proxy/policy_hook.rs` are gone.
- No production code imports `crate::net::policy::NetworkPolicy`.
- DNS and MITM constructors accept only the V2 policy handle.
- Focused policy/DNS/MITM/process tests pass.

## Implementation Notes

- Deleted the legacy `net::policy` module, standalone `net::policy_hook`
  module, MITM `policy_hook` module, and their tests.
- Removed the legacy MITM/DNS/VM/process `policy` handles. Runtime policy is
  now the shared `PolicyConfig` handle used by Policy.
- Reworked DNS cache tests and handler tests so block/rewrite/ask/allow behavior
  is expressed with `policy.dns.*` rules. Cache hits are only reachable after
  Policy evaluation for each query.
- Reworked MITM unit and integration harnesses so HTTP block/allow/hot-reload
  behavior uses `policy.http.*` rules instead of V1 domain rules or
  `http_upstream_ports`.
- Updated comments that still pointed at the removed V1 policy runtime.

## Verification

- `cargo check -p capsem-core -p capsem-process`
- `cargo test -p capsem-core --all-targets --no-run`
- `cargo test -p capsem-core net::dns:: --lib`
- `cargo test -p capsem-core policy_hot_reload --lib`
- `cargo test -p capsem-core policy_http_ --lib`
- `cargo test -p capsem-core --test mitm_integration mitm_proxy_plain_http_denies_disallowed_host`
- `cargo test -p capsem-core --test mitm_integration mitm_proxy_plain_http_denies_port_not_in_allowlist`
