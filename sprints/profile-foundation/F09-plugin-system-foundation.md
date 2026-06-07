# F09 - Plugin System Foundation

## Goal

Add the security plugin system for deterministic enforcement, remote decisions,
observer plugins, and remote alert emission.

## Scope

- Signed plugin bundle identity and replay invariants.
- Remote enforcement decision and observer plugin contracts.
- WASM/TypeScript authoring direction if selected.
- Deterministic `SecurityEvent -> SecurityEvent` transforms.
- Declarative mutations validated by Rust before runtime application.
- Remote alert payloads for plugin decisions, detection findings, observer
  exports, timeouts, and failures.

## Acceptance Criteria

- Same plugin hash plus same input event hash gives same output event hash.
- Plugins cannot mutate immutable event identity, subject, context, or trace.
- Plugin failures fail closed where enforcement is authoritative.
- Remote decisions and observer alerts are written as resolved-event steps or
  linked alert records with bounded, redacted payloads.
