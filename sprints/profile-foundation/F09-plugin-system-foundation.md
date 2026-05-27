# F09 - Plugin System Foundation

## Goal

Add the extension system for deterministic enforcement and observer plugins.

## Scope

- Signed plugin bundle identity and replay invariants.
- Remote enforcement and observer plugin contracts.
- WASM/TypeScript authoring direction if selected.
- Deterministic `SecurityEvent -> SecurityEvent` transforms.
- Declarative mutations validated by Rust before runtime application.

## Acceptance Criteria

- Same plugin hash plus same input event hash gives same output event hash.
- Plugins cannot mutate immutable event identity, subject, context, or trace.
- Plugin failures fail closed where enforcement is authoritative.
