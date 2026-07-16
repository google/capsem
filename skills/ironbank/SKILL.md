---
name: ironbank
description: Use when Capsem VM, network, model, MCP, credential broker, security, package-manager, doctor, benchmark, or release-gate behavior needs black-box acceptance proof
---

# Ironbank

Ironbank is Capsem's full black-box ledger discipline. Use it for release,
VM, network, model, MCP, credential broker, package-manager, doctor,
benchmark, and security acceptance work.

## Core Rule

Do not look at Rust/product internals to decide expected behavior. Ironbank
tests are written from public contracts, CLI help, docs, route responses,
generated schemas, hermetic fixture definitions, logs, DB rows, and installed
package metadata. If the contract is missing, write the RED test for the
missing contract.

### Ironbank parity rule

The Ironbank parity rule is that every portable release gate belongs in
`just test`. The exact candidate must pass the complete recipe locally, then
exact-SHA CI must run the same recipe; green split jobs do not replace it.
Specialized workflows must reuse the same checked-in entrypoints exercised by
`just test`, including workspace/runtime tests, coverage floors,
`capsem-doctor`, Ironbank acceptance, benchmarks, artifact checks, all web
surfaces, and Docker/systemd Linux install plus a real guest-shell proof. Only
unavoidable platform boundaries may remain outside, and each must be named
with its authoritative final gate.

`just test` is the strict superset of portable CI work. CI workflows may run a
smaller relevant slice, but no portable artifact may be built only in workflow
YAML. In particular, VM asset publication uses the same `just build-kernel`
and `just build-rootfs` primitives owned by `just test` through
`just test-assets`; the canonical gate rebuilds every profile for arm64 and
x86_64, validates every required artifact and manifest, and boots each rebuilt
host-architecture image to a guest-shell marker. Input-contract tests are not
a substitute for testing the artifact that was actually built.

## Required Shape

- Suite home: `tests/ironbank/`.
- Runner: Python black-box tests through Capsem, `capsem-doctor`, VM sessions,
  hermetic local services, UDS routes, HTTP routes, logs, and SQLite ledgers.
- One deterministic stimulus asserts the full path: client result, parsed
  facts, CEL/security decision, detection/enforcement rows, protocol rows,
  structured logs, status counters, UDS route, HTTP route, and UI JSON shape.
- Every emitted field is exact-value asserted, typed-invariant asserted, or
  explicitly marked not applicable.
- Unknown DB/log/route fields fail the test until the field ledger is updated.

## Forbidden

- Rust parser/unit proof as an Ironbank gate.
- Public-network dependencies.
- Mocks of the Capsem path.
- Fallback routes.
- Status-code-only replay.
- Row-exists checks.
- `skip`, `skipif`, `slow`, optional markers, or manual OAuth/client dances as
  release proof.

## Package Managers

Installing is not proof. For apt, npm, uv, pip, node, or profile package
rails, assert binary presence/version/hash where relevant and run a command
that proves the package does its job. Example: `zstd` must compress and
decompress known bytes and match the original.
