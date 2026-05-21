# S14 - Rules UI Components

## Goal

Replace raw CEL as the primary rules UI with one reusable rule editor/renderer
that powers per-type rule blocks.

## Dependency On S08a

[S08a - Rule Abstraction And Detection Architecture](S08a-rule-abstraction-detection-architecture.md)
decides that this sprint edits synchronous Capsem policy rules. Detection packs
and findings are separate: S14 may render detection references/finding badges
when available, but the primary editor writes `capsem.policy-pack.v1` policy
rules and never edits Sigma YAML as if it were enforcement policy.

## Tasks

- Build a **single shared rule editor** component (not one editor per rule type).
- Build a **single shared rule renderer/list item** component used by every
  rules block.
- Build per-type visual rule blocks on Profile > Security for:
  - DNS rules
  - HTTP rules
  - Model rules
  - MCP rules
- Each per-type block must include:
  - list existing rules for that type
  - add-rule action scoped to that type
  - edit/delete (or delete-disabled for locked rules) via the shared editor
  - empty-state and locked/provenance display
  - managed/owned rule state with explicit "managed by <setting>" label
    (for example `AI Providers > OpenAI`, `Registry Access > npm`)
- Wire type-specific field/function suggestions into the shared editor via
  configuration (callback/type adapter), not duplicated components.
- Add autocomplete for fields, operators, functions, constants, connectors, MCP
  tools, providers, domains, and profile-scoped objects.
- Cover full decision/action support (`allow|ask|block|rewrite`) and rewrite
  config validation/error rendering.
- Keep raw CEL as advanced escape hatch only.
- Show detection-originated policy suggestions as suggestions that open the
  policy editor prefilled; saving them creates policy rules, not detections.

## Coverage Ledger

- Unit/contract: shared editor/renderer tests and per-type adapter contract
  tests.
- Functional: each rules block can list and add rules; edits round-trip through
  the shared editor; generated/owned rules show managed-by labels and cannot be
  edited.
- Adversarial: invalid expressions, callback/type mismatches, and locked rule
  edits.
- E2E/VM: not primary.
- Telemetry: not primary.
- Performance: autocomplete remains responsive and rule-block rendering remains
  responsive with larger rule sets.
