# S06 Resolver Engine Contract

## Purpose

Define the canonical resolver behavior for profile inheritance, corp restriction
enforcement, and provenance/override tracing.

Rule-shape authority for v1 is S04:

- canonical path: `security.rules.<type>.<rule_name>`
- canonical default profile rule priority: `1`
- capability-derived rules remain generated and locked provenance entries.

## Required Schema Additions

- `Profile.extends_profile_id: Option<String>`
  - Optional parent reference by profile id.
  - Validated as a known profile id at resolve time.
  - Cycle-safe with explicit max chain depth.

## Layered Resolution Model

Resolver must apply layers in strict order:

1. Schema defaults.
2. Ancestor chain from root parent to leaf parent.
3. Selected profile.
4. Corp directives.
5. Derived rules from final capabilities.
6. Final validation gate.

The resolver must be deterministic for identical inputs.

## Corp Directive Semantics

Support the following operations on target paths:

- `add`: add list/map entry.
- `remove`: remove list/map entry.
- `replace`: replace scalar/object value.
- `lock`: mark a path immutable for lower-precedence layers.
- `forbid`: define a denied value/predicate for a path.

If a later layer violates `lock` or `forbid`, resolution fails with a typed
error that includes path, violating value, source layer, and controlling rule.

## Trace Artifact Contract

Resolver writes `vm-effective-trace.json` beside `vm-effective-settings.toml`.

Minimum trace event fields:

- `step`: monotonically increasing integer.
- `path`: dotted settings path.
- `operation`: `set|add|remove|replace|lock|forbid|derive|reject`.
- `source_kind`: `default|profile|corp|derived`.
- `source_profile_id`: optional profile id.
- `source_label`: human-readable source description.
- `before`: prior value (JSON).
- `after`: resulting value (JSON).
- `locked`: bool after this step.
- `reason`: optional explanation.

Trace must allow replay/debug of the final value for any path.
Resolver trace paths for rules must use `security.rules.<type>.<rule_name>`
rather than legacy `security.raw_rules`.

## Runtime Consumption Contract

- `capsem-service` and `capsem-process` must read runtime policy/VM settings
  from resolver output artifacts, not v1 `policy_config`.
- Missing/corrupt resolver artifacts must fail closed with actionable errors.

## Verification Matrix

- Unit:
  - parent-chain resolution order
  - cycle detection and depth guard
  - lock/forbid enforcement
  - deterministic output hash for repeated runs
  - trace event correctness
- Functional:
  - profile select/fork/update paths produce expected effective settings + trace
  - corp directives alter/lock paths as expected
- Adversarial:
  - unknown parent id
  - inheritance cycles
  - forbidden overrides
  - corrupt/missing effective or trace artifacts
- E2E/VM:
  - launch VM with inherited profile chain and corp restrictions
  - verify process behavior reflects resolver output
  - verify debug/status exposes traceable provenance
