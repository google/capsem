# S06 - Assembly And VM-Effective Settings

## Goal

Implement a deterministic resolver engine that:

- applies profile inheritance in a strict sequence,
- enforces corp restrictions across inherited/overridden values,
- emits VM-effective settings for runtime consumption, and
- emits a diff-style trace of every applied override and lock decision.

Detailed contract: `sprints/policy-settings-profiles/S06-resolver-engine-contract.md`.

Prerequisite sprint: `sprints/policy-settings-profiles/S06-pre-network-contract-and-confirm.md`.
Companion sprint: `sprints/policy-settings-profiles/S06a-model-request-rewrite-support.md`.

## Tasks

- [x] Initial profile-level effective settings persistence exists.
- [x] Initial provenance per effective section/rule exists.
- [ ] Add explicit parent linkage to profile schema (`extends_profile_id`).
- [ ] Add parent-chain validation (missing parent, cycle detection, max depth).
- [ ] Add layered resolver pipeline with ordered apply stages.
- [ ] Implement corp operations for policy governance:
  add/remove/replace/lock/forbid.
- [ ] Add lock-aware override enforcement (reject forbidden user overrides).
- [ ] Emit resolver trace artifact with per-path before/after/source
  transitions (diff-style).
- [ ] Persist both runtime artifact and trace artifact beside each session/VM.
- [ ] Switch service/process runtime policy + VM settings reads to resolver
  output artifacts.
- [ ] Expose resolver trace in status/debug/report surfaces.
- [ ] Add full unit/functional/adversarial/E2E verification for layered
  inheritance + corp restrictions + trace integrity.

## Resolver Contract (Reinvented)

### Inputs

- Service settings (`service.toml`) including governance toggles and profile roots.
- Selected profile id for the session/VM.
- Profile catalog across built-in/base/corp/user roots.
- Parent links declared by profile (`extends_profile_id`) once added.
- Corp governance directives (add/remove/replace/lock/forbid).

### Ordered Apply Stages

1. Resolver starts from schema defaults.
2. Apply ancestor chain root-to-leaf using `extends_profile_id`.
3. Apply selected profile.
4. Apply corp directives on top of inherited+selected state.
5. Validate locked/forbidden paths against requested overrides.
6. Materialize final effective settings + derived rules.
7. Emit trace log containing every transition.

No stage may mutate previous trace history; each stage appends operations.

### Artifacts

- `vm-effective-settings.toml`: runtime-consumed final effective state.
- `vm-effective-trace.json`: ordered operations with
  `path`, `before`, `after`, `source_profile_id`, `source_kind`,
  `operation`, `locked`, and `reason`.

The trace must be sufficient to explain "why this final value exists" for any
path and to show where an override was blocked.

## Implemented Slice

`capsem-core::settings_profiles` now exposes `resolve_effective_vm_settings()`
and VM-effective settings persistence helpers. It resolves the selected profile
or service default profile, carries section-level provenance, emits derived
security capability rules plus raw profile rules with provenance, and can write
the resolved immutable settings to `vm-effective-settings.toml` beside a
session/VM. `capsem-service` now attaches VM-effective settings during both
session provisioning and fork: existing readable attachments are preserved, and
corrupt attachments are regenerated from current service profile roots.

This slice is foundational only; it does not yet implement parent-chain
resolution, corp lock/forbid semantics, or diff-style trace artifacts.

Focused test command:

```sh
cargo test -p capsem-core settings_profiles
cargo test -p capsem-service ensure_vm_effective_settings_
```

Result: 41 focused `settings_profiles` tests passed, including default profile
resolution, missing profile error, raw-plus-derived rule provenance, and
VM-effective settings round-trip persistence/corrupt file handling. Service
focused tests also passed for default VM-effective attachment and corrupt-file
regeneration during session attach.

## Coverage Ledger

- Unit/contract: initial precedence, derived rules, and provenance tests are
  present. Persistence tests are present.
- Functional: single-profile effective settings materialize at the model layer,
  round-trip to the session/VM file contract, and are attached by service
  runtime during provision/fork.
- Adversarial: missing profiles and missing persisted VM-effective settings
  fail clearly; corrupt persisted VM-effective settings fail clearly; corrupt
  on-disk session attachments are regenerated during runtime attach; forbidden
  override semantics are not implemented yet.
- E2E/VM: attachment exists on session directories, but layered inheritance
  resolution and trace-driven consumption are not yet wired end-to-end.
- Telemetry: section/rule provenance is serializable/queryable at the model
  layer; per-step resolver trace is pending.
- Performance: assembly cost measured if it appears hot.
