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
in `release-qualification.yaml` on the exact versioned, untagged candidate
commit before an immutable release tag may be created. `release.yaml` must
verify that successful exact-SHA result before any package build, GitHub
Release creation, channel assembly, or deployment may proceed.

There is one explicit temporary GitHub-hosted exception: the full gate runs on
Linux only because GitHub-hosted macOS cannot provide the nested
Virtualization.framework access required by Capsem and Colima, and the
repository has no physical macOS runner. The qualification-workflow comment
must remain until a physical runner exists. The macOS and Linux package jobs
may fan out only after `release.yaml` verifies the exact qualification result;
macOS still builds, notarizes, installs, and verifies the exact `.pkg` before
publication.

- Never replace `just test` with a hand-picked subset, a coverage-only job, or
  a faster release-specific approximation.
- Never treat a local run, a nearby commit's green run, a run title without a
  matching `headSha`, or an agent's claim that tests passed as release
  evidence. Only the successful remote qualification for the exact candidate
  SHA counts.
- The gate includes audits, lint, frontend, Rust coverage, four-VM parallel
  Python tests, Winterfell/MCP lifecycle tests, IronBank, injection,
  integration, benchmarks, cross-compilation, and Docker/systemd install tests.
- Run the complete `just test` gate exactly once for each candidate in
  `release-qualification.yaml`. Do not rerun or duplicate it after tagging or
  packaging.
- Keep cheap clean-environment bootstrap proofs at the start of `just test` for
  every expensive release harness. In particular, prove the Docker install
  image can create its container-owned Python environment and launch pytest
  before Rust/frontend/VM/package work. This fail-fast proof supplements the
  later complete install E2E; it never replaces or skips it. Guard the ordering
  with contract tests so harness drift fails before a multi-hour release run.
- Exact publishable packages must still be installed on macOS and Linux so the
  native installers and their post-install scripts are proven before
  publication. The public install/channel-switch/upgrade glow-up is then the
  end-to-end test of the deployed release. None of these gates substitutes for
  another.
- Stable and nightly require the same exact-SHA qualification and use the same
  parameterized tagged workflow. Only the selected channel may be updated.
- A failed candidate may receive forward fix commits and be qualified again,
  but it must not receive a final tag, GitHub Release, or channel mutation.

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
