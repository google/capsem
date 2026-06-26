# Capsem Agent Instructions

These instructions are for Codex and other coding agents working in this
repository. They complement `CLAUDE.md`, `GEMINI.md`, and the checked-in
`skills/` directory.

## Load Skills First

Before code changes, load the relevant project skill from `skills/`. For tests
and release gates, load `/dev-testing` and `/ironbank`. For debugging, load
`/dev-debugging`. For architecture changes, load `/site-architecture`.

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
