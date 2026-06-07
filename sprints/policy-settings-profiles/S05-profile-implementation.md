# S05 - Profile Implementation

## Goal

Implement profiles as first-class typed files aligned with the S04 canonical
contract.

## Tasks

- [x] Add base/corp/user profile discovery.
- [x] Add profile parsing and validation.
- [x] Add deterministic precedence and collision errors.
- [x] Add create, fork, update, delete primitives.
- [x] Add built-in "Everyday Work" profile.
- [x] Migrate rule parsing from `security.raw_rules` to
      `security.rules.<type>.<rule_name>`.
- [x] Add `extends_profile_id` schema parse + validation wiring.
- [x] Set canonical profile-rule default priority to `1`.
- [x] Restrict v1 `profile_type` surface to `everyday-work|coding`.
- [x] Add malformed/error-path tests for canonical rewrite misuse.

## Implemented Slice

`crates/capsem-core/src/settings_profiles/mod.rs` now includes the first typed
profile model, default icon behavior, profile sections for AI, MCP/connectors,
skills, VM settings, security capabilities, canonical
`security.rules.<type>.<rule_name>` tables, profile discovery across
base/corp/user roots, duplicate-id errors, and user create/update/delete/fork
primitives. The current slice also adds `extends_profile_id` parse/validation,
enforces v1 profile-type scope (`everyday-work|coding`), updates default
profile rule priority to `1`, and emits canonical rule provenance paths in
effective settings (`security.rules.<type>.<rule_name>`).

Current gap versus S04: runtime callback/field normalization and
`model.request` rewrite parity remain in policy-engine follow-up sprints
(S06-pre/S06a); the profile parser now uses canonical `security.rules` tables
with callback/type and rewrite validation.

Focused test command:

```sh
cargo test -p capsem-core settings_profiles
```

Current result: 51 focused `settings_profiles` tests passed, including
`extends_profile_id` validation, legacy profile-type rejection, profile-rule
priority default checks, canonical rule-table parsing and invalid rule-name
rejection, callback/type mismatch rejection (including `dns.query` rejection),
rewrite-target/value validation and capture-reference checks, MCP dotted
`arguments.<path>` parser coverage, derived-rule locked-provenance coverage,
duplicate profile rejection, user CRUD, fork, and governance denial.

## Coverage Ledger

- Unit/contract: parse/validate/discovery/basic CRUD tests are present;
  canonical profile parser contract tests are present.
- Functional: create/fork/update/delete works on user profile files.
- Adversarial: invalid canonical rule names, duplicate profile ids, and disabled
  user profile creation/fork/delete are covered; bad ids, bad icons, duplicate
  skills, bad connector references, duplicate creates, and missing updates are
  covered. Canonical rewrite misuse tests are covered; locked base/corp
  mutation coverage remains.
- E2E/VM: not until S06/S18.
- Telemetry: profile provenance fields available.
- Performance: discovery is deterministic; scale testing remains.
