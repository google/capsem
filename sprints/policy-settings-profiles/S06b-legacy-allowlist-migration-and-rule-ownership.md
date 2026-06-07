# S06b - Legacy Allowlist Migration And Rule Ownership Locks

## Goal

Port legacy allowlist-style policy sources (AI/provider/registry/other legacy
builders) into the canonical profile rule system and enforce DRY rule ownership:
rules derived from another profile setting must be marked uneditable and show
the owning setting name/path.

## Why This Sprint Exists

- Legacy allowlists still exist in old policy assembly paths.
- Equivalent logic must move into canonical `security.rules.<type>.<rule_name>`
  so there is one rule system.
- When a non-rule profile setting generates policy behavior (for example,
  AI provider toggles), the resulting rule must be treated as generated/owned,
  not hand-editable.

## Scope

- Legacy-to-canonical migration:
  - Inventory all legacy allowlist sources (AI, registry/domain/network,
    repository/provider and similar policy builder outputs).
  - Produce a concrete candidate rule-port list and mapping plan.
  - **Pause for explicit user confirmation** of the rule-port list before
    implementing migration changes.
  - Map each source to canonical `security.rules` equivalents.
  - Remove/disable duplicate legacy-path generation once parity is proven.
- Rule ownership model:
  - Add ownership metadata for generated rules:
    - `owner_setting_path` (for example `ai.providers.openai.enabled`)
    - `owner_setting_label` (human-readable source name)
    - `editable = false` for generated/owned rules
  - Keep provenance/trace data pointing back to ownership source.
- Conflict + DRY enforcement:
  - Prevent direct edits to generated rules in API/UI/CLI.
  - If a user attempts to edit a generated rule, return actionable error
    indicating source setting to modify instead.
  - Ensure no duplicated rule logic across raw profile rules and generated
    setting-derived rules.
- Surfaces:
  - UDS/HTTP/CLI payloads include ownership metadata.
  - Debug/status show ownership source for generated rules.
  - UI displays uneditable state and "managed by <setting>" labeling.

## Tasks

- [ ] Produce migration matrix from legacy allowlists to canonical rule entries.
- [ ] Present the full candidate port list to user and receive explicit
      confirmation before implementation starts.
- [ ] Implement generation path into canonical `security.rules` with stable IDs.
- [ ] Add rule ownership metadata fields and serialization.
- [ ] Enforce non-editable behavior for generated/owned rules across backend
      mutation surfaces.
- [ ] Add clear error messages directing users to owning setting.
- [ ] Update resolver/provenance/trace output to include ownership fields.
- [ ] Update UI contracts for locked + managed-by rendering.
- [ ] Add parity tests to prove legacy behavior is preserved via canonical rules.

## Verification Gate

Run after implementation:

```sh
cargo test -p capsem-core -p capsem-service -p capsem-process
```

Pre-implementation checkpoint (required):

- User-approved confirmed list of legacy rules/sources to port is recorded in
  sprint notes before code changes begin.

## Coverage Ledger

- Unit/contract:
  - migration mapping correctness
  - ownership metadata shape and defaults
  - non-editable enforcement for generated rules
- Functional:
  - generated rules appear from owning profile settings
  - rule list/detail surfaces show managed-by source
  - mutation attempts on generated rules fail with source-setting guidance
- Adversarial:
  - duplicate/manual override attempts against generated IDs
  - missing ownership metadata is rejected for generated rules
  - stale legacy + canonical double-application is prevented
- E2E/VM:
  - migrated policy behavior matches legacy allowlist behavior in runtime flows
- Telemetry:
  - audit/debug/status expose owner path/label and provenance
- Performance:
  - generation/resolution remains deterministic with bounded overhead
