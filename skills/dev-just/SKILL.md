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
| `just smoke` | Focused integration gate. |
| `just test` | The only complete release-qualification gate. |

`just --summary` must print only those 11 names.

## What does not belong in Just

- No `test-*` public recipes. Focused tests run their native command directly;
  only `smoke` and `test` are public.
- No `prepare-release`, `qualify-release`, `cut-release`, or `release` recipe.
  Release orchestration belongs to the checked-in workflows and exact
  qualification checks. `just test` owns qualification.
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

Do not fork or approximate this graph in another public recipe.
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
