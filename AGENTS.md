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
  and is reused whole by `just fast-test`, `just test`, ordinary CI, and both
  release lanes. It owns YAML/source syntax, source contracts, Clippy,
  Python/JavaScript checks, web builds, and all dependency audits.
- Release CI calls the same checked-in private test modules but builds only the
  artifact family owned by its lane. Binary CI pulls every selected profile;
  profile CI pulls the selected channel's package. Pulled inputs are verified
  by immutable identity and digest.
- Which lane a run is in is **one indivisible value**, not a set of variables
  each module reads for itself. `capsem.gate.qualification` parses it once, and
  the only legal shapes are local (nothing set), binary release (input
  directory and exact package), and profile release (those plus the profile).
  Every other combination is refused during plan construction. Do not add a
  module that reads `CAPSEM_RELEASE_*` directly — a half-exported environment
  used to build a plausible hybrid that proved source-built bytes in one family
  and manifest-selected bytes in the other.
- `capsem-gate` re-execs under a per-invocation bytecode cache before importing
  any of its own package, and the complete gate refuses to start without the
  marker that says so. A same-size edit inside one timestamp tick otherwise
  leaves a valid-looking `.pyc`, and the source guard digests the bytes on disk
  rather than the bytes being executed.
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

## The gate contract

The justfile dispatches; `src/capsem/gate/` decides. No recipe carries a shell
body and none exceeds five lines, both held by contract tests rather than
convention.

`just test` is **one process, one machine lock, one workspace, one plan** -- 64
steps in a single graph. Both release commands contain that same plan rather
than launching it.

Six rules, each with a guard:

- **A plan action may never invoke `just` or another `capsem-gate` command.**
  `GuardedRunner` refuses it at runtime. The machine lock is not reentrant, so
  every such call was a child waiting out its 7200-second timeout for the lock
  its own parent held. Compose the other command's `fragment` instead.
- **Work is composed from primitives.** `actions` and `fileactions` are the only
  modules that touch the machine, alongside the four that own one piece of
  machine state as their whole purpose. Anything else going around them is work
  the dry run cannot show and the run log cannot time.
- **Ordering is declared, then derived.** A step names what it must follow;
  `graphlib` decides the sequence. A cycle fails before any step runs. Never
  sequence by writing one `plan.add` above another.
- **Contention is declared in `[execution.exclusives]`, with the reason.** Two
  steps can be independent and still unable to share the machine.
- **Teardown is `held(...)`** -- acquired in order, released in reverse,
  evidence preserved before release. A `finally` that removes a directory is a
  `Resource` that was not written.
- **Every value lives in `config/gate.toml`.** No path, filename, architecture
  or channel name in code.

Two more, which follow from the first:

- **A plan describes; it does not act.** `plan()` is built with the machine
  sealed, so `--dry-run` cannot touch it -- and inspection is answered before
  any re-exec, or asking becomes doing.
- **Anything that writes takes the machine lock.** `[execution.exclusives]`
  entries are `threading.Lock`s: they order steps inside one plan and
  coordinate nothing between two `capsem-gate` processes.

One gate runs per machine, enforced by `flock` rather than a pidfile, and every
run is recorded under `target/gate-runs/` and bounded by `[disk]`. The run log
is written by the runner rather than by call sites, so nothing can be forgotten
into invisibility.

Read `/dev-gate` before changing any of it.
