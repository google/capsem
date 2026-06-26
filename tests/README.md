# Capsem Tests Layout

Tests use production source contracts from `config/` only when validating the
real checked-in config. Synthetic inputs and integration fixtures belong under
`tests/fixtures/`.

## Fixtures

- `tests/fixtures/config/` contains test-only settings, corp, profile, and rule
  fixtures. Do not add test fixtures under root `config/`.
- Source profile fixtures should follow the same rule as production profiles:
  no manual asset or sibling-file `hash`/`size` pins unless the fixture is
  explicitly testing materialized runtime config.

## Black-Box Gates

Release-critical VM, security, network, model, MCP, credential, doctor, and
benchmark work owes Ironbank coverage under `tests/ironbank/`. Those tests
exercise public routes and runtime evidence; they must not become parser-only
or Rust-internal proof.

## Citadel Guards

`tests/citadel/` stores architectural regression guards: source-level tests for
mistakes the project has already paid for once. These are not optional lint
nits. When a Citadel guard trips, read the failure text and the linked skill or
contract before editing around it.
