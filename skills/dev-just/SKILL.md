---
name: dev-just
description: Capsem's small Just surface and its boundary with the Python gate. Use when adding or changing a recipe, or deciding whether logic belongs in Just or in a gate plan.
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
| `just dev [ui\|frontend\|tui]` | Select one development surface. No passthrough arguments: `just` joins a variadic before interpolating it, so no spelling preserves argument boundaries. Use `uv run --project build_system --frozen capsem-gate dev tui …` when you need them. |
| `just build [debug\|release]` | Build the desktop app with its embedded frontend. |
| `just build-all [debug\|release]` | Build all host binaries, desktop app, docs, and site. |
| `just build-docs` | Build documentation and marketing sites. |
| `just shell` | Start the service and enter a temporary VM. |
| `just exec "<command>"` | Run one command in a fresh temporary VM. |
| `just run-service` | Materialize assets/config and start the local daemon idempotently. |
| `just logs [sandbox-id\|failure]` | Tail service logs, show a sandbox log, or list the latest preserved failure evidence. |
| `just doctor [fix]` | Validate host tools, Docker/Colima, Tart cache/boot/SSH, signing, and assets. |
| `just fast-test` | Explicitly incomplete source feedback; it prints the targeted and release rails. |
| `just focus-test <group> [reuse\|clean]` | Rerun one existing owner: assets, binaries, benchmark, install, release-system, or functional. `release-system` is source-only and needs no package or install. |
| `just install` | Optional hands-on local package testing; never a release prerequisite and never release authority. |
| `just test-full [source-commit]` | Exceptional cold complete diagnostic; never the routine edit loop. |
| `just release-binaries <channel> <source-commit>` | Dispatch qualification and publication of packages against pulled profiles. |
| `just release-profile <channel> <profile> <source-commit>` | Dispatch qualification and publication of one profile against the pulled package. |

`just --summary` must print exactly the names in `[just].approved` and nothing
else. The count is not repeated here on purpose: this line used to say "those
13 names", which was wrong the moment the surface legitimately grew, and a
number restated away from its source is a number that goes stale silently.
`config/public-surface.toml` is the authority; the contract test compares the
live recipe list against it.

## The Python system replaced shell orchestration

Treat the Justfile as a stable user interface, not as the implementation of a
command. For `focus-test`, `release-binaries`, and `release-profile`, one recipe
line crosses one exact argv boundary into `capsem-gate`; `install` prints its
hands-on-only warning before crossing the same boundary. From there:

| Concern | Owner |
|---|---|
| Public command name, defaults, and exact argv dispatch | `justfile` plus `config/public-surface.toml` |
| Command shape and plan graph | `src/capsem/gate/<domain>.py` |
| Shared work | a composable `fragment(...)`, deduplicated with `plan.shared(...)` when necessary |
| Ordering | explicit graph edges passed through `after=` |
| Subprocess or filesystem work | `actions.py`, `fileactions.py`, and their typed domain actions |
| Always-run setup, teardown, and failure preservation | `Resource` implementations held by the command |
| Paths, filenames, environment names, channels, architecture values | `config/gate.toml` |
| Release mode and manifest-selected artifact paths | one `Qualification` value parsed by the command |
| Evidence and timing | the gate run log written by the execution funnel |

The old system expressed orchestration through shell bodies, Just dependencies,
textual order, nested recipes, ambient environment reads, and ad hoc cleanup.
Do not use remaining private recipes as templates for new logic. Move each
decision into the Python graph, where dry-run, graph inspection, run logging,
locking, teardown, and contract tests see it.

A plan action must never invoke `just` or another `capsem-gate` command. Compose
the other command's fragment instead. The machine lock is not reentrant, so a
nested gate waits for the lock held by its own parent. Likewise, do not split a
release with an operator-authored receipt. The complete candidate owns one
process, lock, workspace, plan, and runner journal; release revalidates that
content-addressed journal before its short publication plan.

Read `/dev-gate` before changing Python orchestration and `/release-process`
before changing either release plan.

## What a recipe may contain

A recipe is a dispatch or a single command. Nothing else, and this is checked
rather than advised:

- no shell body (no `#!/bin/bash`) -- `build_system/tests/gate/test_gate_boundary.py`
- at most five executable lines
- no `if`, `for`, `while`, `case`, `until` or `trap`

The old justfile carried roughly 2070 lines of inline `bash` across thirty-five
recipes, none of it reachable by a test, so every defect in it was found by
running the forty-minute gate. The ratchet is what matters: do not add shell
orchestration back. Logic lives in `src/capsem/gate/`; see `/dev-gate` for how
to add or change a command.

The one exception is a single command with no branching -- `cargo build`,
`cd web/app && pnpm run dev` -- where routing through Python would add a `uv`
startup and, for an interactive dev server, break TTY and signal handling, in
exchange for no decision made.

## What does not belong in Just

- `fast-test` is exactly the incomplete `test-fast` module. It may not bundle
  compiled, VM, install, or release work.
- `focus-test` aliases an existing owning gate command; it must not copy or
  compose a second test graph. `focus-test release-system` aliases the
  source-only release-contract owner; package rehearsal and installed-product
  proof belong to qualification. Neither feedback command is release authority.
- No generic or combined release recipe. The two approved release commands
  revalidate exact qualification before delegating to one checked-in
  implementation, and the two workflows share the per-channel lock.
- No dependency-update, fixture-update, audit-only, coverage-only,
  cleanup, session-SQL, or extra package-install convenience recipes. Call the owning
  script/tool directly.
- No separate UI aliases. Use `just dev <surface>` or `just build`.
- No public build primitives for kernel, rootfs, Docker images, architectures,
  or package rails.
- No public continuation recipe. Exact-commit `just test-full` derives a partial
  frontier only from its archived event graph and retained full-SHA prefix;
  working-tree diagnostic continuation is not qualification. Both release
  commands refuse continuation flags.

Private underscore recipes may exist only as dependencies of the approved
commands or as narrow CI primitives. Specialized skills and workflows may
name those internals, but general developer guidance must not present them as
public commands. Prefer a tested script when orchestration has state,
branching, reporting, cleanup, or resource ownership.

## Canonical testing

`just test-full` owns the complete graph:

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
`just test-full` and `just fast-test`; it owns YAML/source syntax, source contracts,
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
uv run --project build_system --frozen python build_system/scripts/audit/check_public_surface.py
uv run --project build_system --frozen python -m pytest -c build_system/pyproject.toml --rootdir . tests/test_public_surface_contract.py
```

The gate also locks the Capsem CLI command tree and service HTTP method/path
table. Review `config/public-surface.toml` as an API approval ledger, never as a
snapshot to refresh automatically.
