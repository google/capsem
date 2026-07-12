# Capsem Agent Instructions

These instructions are for Codex and other coding agents working in this
repository. They complement `CLAUDE.md`, `GEMINI.md`, and the checked-in
`skills/` directory.

## Load Skills First

Before code changes, load the relevant project skill from `skills/`. For tests
and release gates, load `/dev-testing` and `/ironbank`. For debugging, load
`/dev-debugging`. For architecture changes, load `/site-architecture`.

## Release CI Is the Authority

Every stable and nightly binary release must run the complete `just test` gate
on both macOS and Linux, in parallel inside the same globally serialized
release workflow, before any package build, GitHub Release creation, channel
assembly, or deployment may proceed.

The macOS gate requires a physical Apple-silicon self-hosted runner labeled
`self-hosted`, `macOS`, `ARM64`, and `capsem-release`. GitHub-hosted macOS
runners cannot provide the nested Virtualization.framework access required by
Capsem and Colima. Never change the gate back to a hosted macOS runner or bypass
the missing physical runner.

- Never replace `just test` with a hand-picked subset, a coverage-only job, or
  a faster release-specific approximation.
- Never treat a local run, a prior tag/commit's green run, or an agent's claim
  that tests passed as release evidence. Only the current immutable release
  tag's CI gate counts.
- The gate includes audits, lint, frontend, Rust coverage, four-VM parallel
  Python tests, Winterfell/MCP lifecycle tests, IronBank, injection,
  integration, benchmarks, cross-compilation, and Docker/systemd install tests.
- Run the complete `just test` gate exactly once per operating system in each
  release workflow. Do not duplicate it after packaging.
- Exact publishable packages must still be installed on macOS and Linux so the
  native installers and their post-install scripts are proven before
  publication. The public install/channel-switch/upgrade glow-up is then the
  end-to-end test of the deployed release. None of these gates substitutes for
  another.
- Stable and nightly use the same parameterized workflow and the same two-OS
  gate. Only the selected channel may be updated.

## Logger DB Boundary

Telemetry and security ledgers are database-owned.

- Service routes, UI handlers, MCP helpers, and benchmark harnesses must not
  call `rusqlite::Connection::open` or `DbReader::open` directly.
- They must not create service-owned logged-data projection caches.
- They may own query intent, but the logger DB object owns query execution.
- `capsem-logger` owns SQLite connection threads, `mem`/disk table layout,
  batching, flushing, rehydration, WAL tuning, and future FTS5/search.
- Do not hardcode route-specific query helpers in `DbWriter` as a substitute
  for this boundary. The DB object is an execution/storage owner, not a route
  semantics registry.
- `write(event).await` means the event was accepted into the DB-owned producer
  buffer. Tests that need read-after-write visibility must use the DB flush
  barrier or shutdown/reopen; route code must not sleep, poll, or build a
  projection cache to make ledger rows appear.
- Empty table means empty result. Missing table or column means broken schema
  and must fail loudly; do not add compatibility branches that treat missing
  ledger shape as empty data.

Every change touching logged data needs tests that guard this boundary.
