# S16 - Profile UI

## Goal

Make profiles first-class in the UI.

## Tasks

- Add profile selector.
- Add create, fork, delete flows.
- Show icon, name, description, best-for, type, version.
- Add General, Appearance, AI Providers, MCP & Connectors, Skills, VM, Security.
- Make session launch use selected/default profile.

## Coverage Ledger

- Unit/contract: profile UI model tests.
- Functional: create/fork/delete/select tests.
- Adversarial: locked/forbidden profile actions.
- E2E/VM: launch session with selected profile.
- Telemetry: not primary.
- Performance: profile switching remains responsive.
