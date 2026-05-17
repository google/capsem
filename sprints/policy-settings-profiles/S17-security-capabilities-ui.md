# S17 - Security Capabilities UI

## Goal

Build Profile > Security around capabilities first, canonical rule editing
second.

## Tasks

- Add capability controls using reusable settings components.
- Cover credential brokerage, PII, MCP retrieval/RAG, MCP/local tools,
  network/domain/HTTP, model scanning, file boundaries, and audit posture.
- Integrate S14 shared rule editor/renderer inside per-type rule blocks
  (DNS, HTTP, Model, MCP) under the capability controls.
- Ensure each per-type block supports list existing rules + add rule for that
  type while preserving locked/provenance display.
- Show generated rules from non-rule settings (capabilities, AI provider
  toggles, registry access controls, etc.) as uneditable with "managed by
  <setting>" source label.
- Show generated gray rules and locked inherited rules.
- Link provenance back to source capability/setting.

## Coverage Ledger

- Unit/contract: capability-to-rule display tests.
- Functional: capability control tests plus per-type rule block interaction
  tests using the shared S14 components.
- Adversarial: locked inherited rules and invalid canonical rule input.
- Adversarial: managed-by generated rules cannot be edited directly and show
  actionable source-setting guidance.
- E2E/VM: capability change enforces through profile.
- Telemetry: audit posture display.
- Performance: not primary.
