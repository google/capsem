---
name: dev-just
description: Capsem's deliberately small Just command surface. Use when choosing, changing, documenting, or reviewing a Just recipe.
---

# Capsem Just discipline

The Justfile is a product interface, not a script drawer. Its public surface is
exactly the allowlist in `config/public-surface.toml`; the contract test derives
the live recipe list and fails on additions, removals, renames, or count drift.
A new public recipe requires explicit user/product approval and an intentional
allowlist update in the same change.

## Approved public commands

| Command | Contract |
|---|---|
| `just dev [ui\|frontend\|tui]` | Select one development surface. |
| `just build [debug\|release]` | Build the desktop app with its embedded frontend. |
| `just build-all [debug\|release]` | Build all host binaries, desktop app, docs, and site. |
| `just build-docs` | Build documentation and marketing sites. |
| `just shell` | Start the service and enter a temporary VM. |
| `just exec "<command>"` | Run one command in a fresh temporary VM. |
| `just run-service` | Materialize assets/config and start the local daemon idempotently. |
| `just logs [sandbox-id\|failure]` | Tail service logs, show a sandbox log, or list the latest preserved failure evidence. |
| `just doctor [fix]` | Validate host tools, Docker/Colima, Tart cache/boot/SSH, signing, and assets. |
| `just smoke` | Focused developer integration feedback; never release qualification. |
| `just test` | Complete local all-artifact construction and test proof. |
| `just release-binaries <channel>` | Run complete `just test`, then build and release only packages for one channel against pulled profiles. |
| `just release-profile <channel> <profile>` | Run complete `just test`, then call `capsem-admin release` for one profile against the pulled package. |

`just --summary` must print only those 13 names.

## What a recipe may contain

A recipe is a dispatch or a single command. Nothing else, and this is checked
rather than advised:

- no shell body (no `#!/bin/bash`) -- `tests/test_gate_boundary.py`
- at most five executable lines
- no `if`, `for`, `while`, `case`, `until` or `trap`

The justfile carried roughly 2070 lines of inline `bash` across thirty-five
recipes, none of it reachable by a test, so every defect in it was found by
running the forty-minute gate. It is 73 body lines now. Logic lives in
`src/capsem/gate/`; see `/dev-gate` for how to add or change a command.

The one exception is a single command with no branching -- `cargo build`,
`cd frontend && pnpm run dev` -- where routing through Python would add a `uv`
startup and, for an interactive dev server, break TTY and signal handling, in
exchange for no decision made.

## What does not belong in Just

- `smoke` is the one public focused developer gate. It is never sufficient for
  release; both release commands must call complete `test`, not `smoke`.
- No generic or combined release recipe. The two approved release commands
  each run `just test` before delegating to one checked-in implementation, and
  the two workflows share the per-channel lock.
- No dependency-update, fixture-update, audit-only, coverage-only, benchmark,
  cleanup, session-SQL, or package-install convenience recipes. Call the owning
  script/tool directly.
- No separate UI aliases. Use `just dev <surface>` or `just build`.
- No public build primitives for kernel, rootfs, Docker images, architectures,
  or package rails.

Private underscore recipes may exist only as dependencies of the approved
commands or as narrow CI primitives. Specialized skills and workflows may
name those internals, but general developer guidance must not present them as
public commands. Prefer a tested script when orchestration has state,
branching, reporting, cleanup, or resource ownership.

## Canonical testing

`just test` owns the complete graph:

- fail-fast bootstrap and clean install-harness proof;
- audits, lint, frontend, Rust and Python coverage;
- both profile/architecture VM asset lanes and real VM boot;
- four-VM parallel integration;
- Linux parity and both `.deb` architectures;
- host package SBOM;
- Linux systemd install plus channel glow-up;
- on macOS, an unsigned local `.pkg` install in Tart, ad-hoc signature checks
  on the installed executable payload, and physical Apple VZ boot from that
  exact package.

Release CI calls the checked-in `_test-fast`, `_test-static`,
`_test-artifacts`, `_test-functional`, `_test-glowup`, and
`_test-release-contracts` modules. `_test-fast` is also the first phase of
`just test` and `just smoke`; it owns YAML/source syntax, source contracts,
Clippy, Python and JavaScript checks, and every locked-ecosystem vulnerability
audit. Callers must reuse it whole rather than duplicating a subset.
Binary CI builds packages and pulls profiles; profile CI builds one profile and
pulls packages. Both retain complete functional and glow-up proof before
activation. Do not fork or approximate this graph in another public recipe.
All checked-in automation enters through the same two public release recipes;
it must not call their scripts or workflows directly.
Local qualification must not import, unlock, or use Apple Developer
certificates. Developer ID package signing, notarization, and stapling belong
only to the tagged publication workflow.

## Public-surface gate

Run:

```bash
uv run python scripts/check_public_surface.py
uv run python -m pytest tests/test_public_surface_contract.py
```

The gate also locks the Capsem CLI command tree and service HTTP method/path
table. Review `config/public-surface.toml` as an API approval ledger, never as a
snapshot to refresh automatically.
