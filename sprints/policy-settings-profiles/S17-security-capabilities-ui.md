# S17 - Security Capabilities UI

## Goal

Build Profile > Security around capabilities first, canonical rule editing
second.

## Tasks

- Add capability controls using reusable settings components.
- Cover credential brokerage, PII, MCP retrieval/RAG, MCP/local tools,
  network/domain/HTTP, model scanning, file boundaries, and audit posture.
- Integrate S14 shared enforcement rule editor/renderer inside per-type rule blocks
  (DNS, HTTP, Model, MCP) under the capability controls.
- Integrate S14 detection rule/finding/backtest views as a separate detection
  surface. Detection can suggest enforcement changes, but it does not edit
  enforcement directly.
- Ensure each per-type block supports list existing rules + add rule for that
  type while preserving locked/provenance display.
- Show generated rules from non-rule settings (capabilities, AI provider
  toggles, registry access controls, etc.) as uneditable with "managed by
  <setting>" source label.
- Show generated gray rules and locked inherited rules.
- Link provenance back to source capability/setting.
- Add backtest affordances for enforcement and detection before saving or
  enabling a rule/pack. The default UI renders the service's 100 matched-event
  sample with evidence diversity and full local matched evidence.

## Coverage Ledger

- Unit/contract: capability-to-rule display tests and backtest-result rendering
  tests.
- Functional: capability control tests plus per-type rule block interaction
  tests using the shared S14 components.
- Adversarial: locked inherited rules, invalid canonical rule input,
  unsupported detection/Sigma input, and mismatched backtest expected labels.
- Adversarial: managed-by generated rules cannot be edited directly and show
  actionable source-setting guidance.
- E2E/VM: capability change enforces through profile.
- Telemetry: audit posture display plus enforcement/detection stats display.
- Performance: not primary.
