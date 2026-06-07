# S14 - Rules UI Components

## Goal

Replace raw CEL as the primary enforcement UI with reusable rule editor,
renderer, backtest, and finding components that respect the split between
enforcement and detection.

## Dependency On S08a

[S08a - Rule Abstraction And Detection Architecture](S08a-rule-abstraction-detection-architecture.md)
decides that this sprint edits synchronous Capsem enforcement rules. Detection
packs and findings are separate: S14 may render detection references, finding
badges, detection backtest results, and detection suggestions, but the primary
enforcement editor writes enforcement CEL rules and never edits Sigma YAML as
if it were enforcement policy.

S08b adds the canonical policy context ABI that this UI must mirror. The editor
suggests typed roots such as `http.request.host`, `http.request.header(name)`,
`mcp.request.tool_name`, `model.request.provider`, `file.activity.path_class`,
and `process.activity.command_class`. It must not suggest or accept `event.*`
as a public field path.

## Tasks

- Build a **single shared enforcement rule editor** component (not one editor
  per rule type).
- Build a **single shared rule renderer/list item** component used by every
  enforcement and detection block.
- Build a shared backtest result table/card component for enforcement and
  detection. Default view shows summary counts plus up to 100 matched events
  returned by the service, preserving event refs and full local evidence.
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
  configuration from the shared policy-context schema, not duplicated
  components or hand-authored UI-only lists.
- Add autocomplete for fields, operators, functions, constants, connectors, MCP
  tools, providers, domains, and profile-scoped objects.
- Cover full decision/action support (`allow|ask|block|rewrite`) and rewrite
  config validation/error rendering.
- Keep raw CEL as advanced escape hatch only, still validated against
  canonical roots and still rejecting `event.*`.
- Show detection-originated policy suggestions as suggestions that open the
  policy editor prefilled; saving them creates policy rules, not detections.
- Add detection pack/rule list and backtest views that call `/detection/*`.
  Detection edit/import UX may support Sigma YAML, but must not present Sigma
  as a blocking rule language.
- Add enforcement backtest actions that call `/enforcement/backtest` before a
  rule is installed or saved.
- Full matched evidence is visible in local UI backtest/hunt results by
  default; redaction belongs to export/support-bundle flows.

## Coverage Ledger

- Unit/contract: shared editor/renderer tests, backtest result component tests,
  and per-type adapter contract tests.
- Functional: each enforcement block can list and add rules; edits round-trip
  through the shared editor; generated/owned rules show managed-by labels and
  cannot be edited. Detection blocks list findings/rules and can backtest
  candidate detection content without turning it into enforcement.
- Adversarial: invalid expressions, `event.*` paths, callback/type mismatches,
  and locked rule edits; unsupported Sigma constructs show typed detection
  diagnostics.
- E2E/VM: not primary.
- Telemetry: finding/rule stats display consumes S08b counters without inventing
  UI-only counters.
- Performance: autocomplete remains responsive and rule-block rendering remains
  responsive with larger rule sets.
