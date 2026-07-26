# Capsem Agent Instructions

These instructions are for Codex and other coding agents working in this
repository. They complement `CLAUDE.md`, `GEMINI.md`, and the checked-in
`skills/` directory.

## Load Skills First

Before code changes, load the relevant project skill from `skills/`. For tests
and release gates, load `/dev-testing` and `/ironbank`. For debugging, load
`/dev-debugging`. For architecture changes, load `/site-architecture`.

## Serialized Orthogonal Releases

The governing contract is `tmp/release-spec.md`. Capsem has exactly two release
commands:

```text
just release-binaries <channel>
just release-profile <channel> <profile>
```

- Each release command itself runs complete `just test` first. There is no
  separate preparation or qualification command. A shared source guard then
  requires and, if necessary, fast-forward-pushes the exact clean `main` HEAD
  that passed; only afterward may stamping, authoring, or dispatch begin.
- Local `just test` remains the complete all-artifact proof. It rebuilds
  packages and every configured profile, then runs audits, lint, frontend,
  Rust/Python coverage, all VM suites, Winterfell/MCP lifecycle, IronBank,
  injection, integration, benchmarks, full `capsem-doctor`, native install,
  and glow-up.
- The private `_test-fast` module runs before Docker/Colima or artifact work
  and is reused whole by `just smoke`, `just test`, ordinary CI, and both
  release lanes. It owns YAML/source syntax, source contracts, Clippy,
  Python/JavaScript checks, web builds, and all dependency audits.
- Release CI calls the same checked-in private test modules but builds only the
  artifact family owned by its lane. Binary CI pulls every selected profile;
  profile CI pulls the selected channel's package. Pulled inputs are verified
  by immutable identity and digest.
- Every pairing that becomes public must pass the complete functional and
  glow-up modules. Saving build time never means skipping tests.
- Binary and profile releases share the workflow-level
  `capsem-release-${channel}` lock from source-manifest resolution through
  production deployment. Different channels remain independent.
- A profile requiring new code is published immutably but remains inactive.
  The following binary release consumes that staged profile without rebuilding
  it and activates only the fully tested compatible graph.
- The manifest is the bible: if an artifact is not selected by it, it does not
  exist for release, update, cache, test, or boot. Fetch mutable manifests
  fresh. Cache immutable bytes only under their manifest-recorded digests,
  independently of channel, and verify every hit before use. Existing SBOM,
  OBOM, attestations, and GitHub logs are the evidence; do not add a parallel
  release ledger or result file.
- All first-party and corporate manifest/profile authoring goes through
  `capsem-admin`. Corporations select official Capsem packages; they do not
  build or replace them.
- Exact publishable packages must be installed on macOS and Linux before
  publication. Public polling, channel switching, binary/profile transitions,
  tamper rejection, Winterfell, and doctor remain mandatory glow-up proof.

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
