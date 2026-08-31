# Capsem Agent Instructions

The one contract for every coding agent in this repository. `CLAUDE.md` and
`GEMINI.md` are symlinks to this file, and
`tests/citadel/test_agent_contract_is_one_file.py` keeps them that way.

They used to be separate files with almost disjoint content, which meant each
agent was held to a different contract: Claude never saw the release rules or
the bounded-diagnostics wrapper, and Codex never saw the code style or the
invariants. Nobody chose that -- the files simply grew apart, and nothing was
comparing them.

## Quick Start

```bash
just doctor        # Check tools (first time)
just doctor fix    # Install prerequisites and materialize missing VM assets
just shell         # Build + boot VM (~10s)
just fast-test     # Incomplete source feedback; prints the next supported rails
just focus-test functional # Rerun one named functional owner
source_commit=$(git rev-parse HEAD)
just test "$source_commit" # Optional complete local proof; exact repeats reuse its journal.

# Optional hands-on local testing; never a release prerequisite
just install

# Release dispatchers are sufficient on their own; hosted lanes self-qualify
just release-binaries nightly "$source_commit"
just release-profile nightly code "$source_commit"
```

See `/dev-just` for the full recipe reference and dependency chains.

## Project Layout

```
crates/capsem-foundation/      Low-level paths, UDS, logging, polling, and IPC handshake
crates/capsem-assets/          Asset manifest compatibility, resolution, download, and verification
crates/capsem-config/          Product config types, parsing, validation, and provider/MCP identity
crates/capsem-credentials/     Credential provider contracts and durable credential store
crates/capsem-core/            VM, hypervisor, security engine, and host network runtime
crates/capsem-service/         Daemon service (axum HTTP over UDS, VM lifecycle)
crates/capsem-process/         Per-VM process (boots VM, bridges vsock, job store)
crates/capsem/                 CLI client (create, shell, exec, list, install, assets, update)
crates/capsem-tui/             Terminal control UI (reads and drives state via the gateway)
crates/capsem-admin/           Profile/asset/release administration (validate, materialize, publish)
crates/capsem-gateway/         TCP-to-UDS HTTP gateway (frontend + tray + remote auth)
crates/capsem-mcp/             Host MCP server for AI agents (stdio, bridges to service)
crates/capsem-mcp-aggregator/  Low-privilege subprocess: connects to external MCP servers
crates/capsem-mcp-builtin/     Stdio MCP server for built-in tools (HTTP, file/snapshot)
crates/capsem-agent/           Guest PTY agent + net-proxy + dns-proxy + mcp-server + sysutil (musl)
crates/capsem-app/             Thin Tauri desktop shell (points at gateway)
crates/capsem-tray/            System tray (polls gateway, quick actions)
crates/capsem-proto/           Shared protocol types (host-guest, service-process IPC)
crates/capsem-logger/          Session DB schema, queries, async writer
crates/capsem-guard/           Companion lifecycle primitives (parent-watch + flock singleton)
crates/capsem-bench/           Benchmark harness, ships as capsem-bench-rs (guest musl + host)
crates/capsem-mock-server/     Hermetic mock upstream (HTTP/TLS/WS) for tests and benchmarks
web/app/                 Astro 7 + Svelte 5 + Tailwind v4 + owned semantic CSS
web/marketing/                     Marketing website (Astro + Svelte 5)
web/docs/                     Documentation site (Astro Starlight)
build_system/builder/      capsem-builder backend and gate implementation
build_system/release_site/ Release channel site generator (Astro, writes target/distribution/)
build_system/scripts/      Thin functional command boundaries for build and release tooling
config/                   Runtime product config source -- never developer skills (see Skills)
config/profiles/<id>/     Profile ledgers (code, co-work): profile.toml + packages, MCP, rules, root seed
guest/artifacts/          Guest scripts and diagnostics (capsem-init, bashrc, tests)
target/assets/            Built VM assets (gitignored, per-arch: target/assets/{arch}/)
web/graphics/             Brand icons and Tauri app icons (source of truth)
skills/                   Shared AI agent skills (SKILL.md format)
tests/                    Cross-crate suites (ironbank/ black-box gates, citadel/ guards)
                          citadel/ is source-level and runs in the fast phase: a
                          recorded mistake must fail before the expensive work, and
                          each guard carries the reason in its failure message
```

## Read the Gate Digest First

`target/gate-runs/DIGEST.md` is the state of the build across recent runs: what
the last run did, which steps keep failing, where the time goes, and what to do
about it. Every gate run regenerates it; `uv run --project build_system --frozen capsem-gate runs digest`
rebuilds it on demand and `uv run --project build_system --frozen capsem-gate runs trend --step <label>` follows
one step run by run.

Read it before starting work and before reporting that anything passes. One
green run says nothing about a step that fails one time in four, and an
intermittent failure is the most expensive kind here precisely because each
sighting looks like bad luck.

## Skills

Skills live in `skills/` at the project root. This is the canonical checked-in
developer skill library. Agent-specific discovery may symlink or copy from this
path; runtime product config must not mirror developer skills under `config/`.

```
skills/<name>/SKILL.md    One skill per directory
```

Prefix-based grouping: `dev-*`, `build-*`, `release-*`, `site-*`, `frontend-*`, `meta-*`. `asset-pipeline` covers the build-to-boot asset flow. See `/meta-organize-skills` for conventions.

**Do not** put skill source files in `.claude/`, `.codex/`, `.gemini/`, or
`config/skills/`. Those roots are agent-local settings or product config, not
the developer skill source.

Before code changes, load the relevant project skill from `skills/`. For tests
and release gates, load `/dev-testing` and `/ironbank`. For debugging, load
`/dev-debugging`. For architecture changes, load `/site-architecture`.

## Skills -- LOAD BEFORE CODING

Skills contain hard-won lessons and project-specific patterns. **Before writing or modifying code, load the relevant skill.** Skipping skills leads to repeated bugs (e.g., blocking async, serde_json::Value on hot paths, missing VM tests).

| Area | Skill | When to load |
|------|-------|--------------|
| Overview | `/dev-capsem` | Orienting on any task, finding which skill to use |
| Quick start | `/dev-start` | First-time bootstrap, onboarding |
| Dev setup | `/dev-setup` | Environment setup, tool install, troubleshooting |
| Rust patterns | `/dev-rust-patterns` | Writing any Rust code in capsem-core/app/agent |
| MITM proxy | `/dev-mitm-proxy` | TLS, HTTP inspection, SSE parsing, ai_traffic |
| MCP | `/dev-mcp` | capsem-mcp server, MCP gateway, aggregator, builtin, tool routing |
| Testing | `/dev-testing` | Running or writing tests, TDD, coverage |
| VM testing | `/dev-testing-vm` | In-VM diagnostics, capsem-doctor, session DB |
| Hypervisor testing | `/dev-testing-hypervisor` | Apple VZ / KVM, VirtioFS, vsock tests |
| Frontend testing | `/dev-testing-frontend` | vitest, svelte-check, visual verification |
| Python testing | `/dev-testing-python` | capsem-builder pytest, coverage, golden fixtures |
| Session DB | `/dev-session-debug` | Inspecting session.db, correlating events |
| Benchmarking | `/dev-benchmark` | capsem-bench, performance regression |
| capsem-doctor | `/dev-capsem-doctor` | In-VM diagnostic suite, adding new tests |
| Frontend | `/frontend-design` | UI components, Svelte 5 runes, Tailwind, owned semantic CSS |
| Build images | `/build-images` | capsem-builder, guest config, rootfs, kernel |
| Initrd repack | `/build-initrd` | Guest binary changes, fast iteration loop |
| Asset pipeline | `/asset-pipeline` | Asset manifest, hash verification, boot-time resolution |
| Just recipes | `/dev-just` | Which just command to run for a given task |
| Build/release gate | `/dev-gate` | Adding or changing a `capsem-gate` command; boundary, primitive, or contention guard failures |
| Citadel guards | `/citadel` | Adding a guard, a linter, or a source surface; a citadel test failing |
| Debugging | `/dev-debugging` | Bug investigation, reproduce-first workflow |
| CI triage | `/dev-ci` | Red gates, pr-gate failures, rerun decisions, stop-the-line policy |
| Sprints | `/dev-sprint` | Running a multi-step feature sprint |
| Release | `/release-process` | CI, signing, notarization, changelog |
| Release gate proof | `/ironbank` | Black-box acceptance proof for VM, network, MCP, security, or release-gate behavior |
| Bug queue | `/dev-bug-review` | Working a queue of bug reports one-by-one (confirm, push back, fix, commit) |
| Installation | `/dev-installation` | Setup wizard, service registration, self-update, install tests |
| Architecture | `/site-architecture` | System design, service architecture, vsock, key files |
| Docs site | `/site-infra` | Writing/editing docs, Starlight, sidebar, release pages |
| Marketing site | `/site-marketing` | Marketing website (capsem.org), copy, components, theme |
| Skills system | `/dev-skills` | How skills work, naming, discovery |
| Skills layout | `/meta-organize-skills` | Skills directory conventions, symlinks |
| Skill discovery | `/meta-find-skills` | Finding or installing skills from the ecosystem |
| Skill authoring | `/meta-skill-creation` | Creating, improving, or evaluating skills |

## Desktop app (capsem-app)

- Thin Tauri webview shell -- only IPC commands are `log_frontend`, `open_url`, `check_for_app_update`. No VM logic, no capsem-core dep. All UI state flows through the gateway at `http://127.0.0.1:19222`.
- **The frontend is embedded in the Rust binary at cargo build time** via `tauri::generate_context!()`. Running `pnpm run build` alone does **nothing** to a compiled binary. After any frontend change meant for the desktop app, run `just build` (frontend build + `cargo build -p capsem-app`). The toolbar shows `build <timestamp>` -- if it's stale, you forgot to rebuild the Rust binary.
- Iframe `src` for bundled pages must be explicit (`/vm/terminal/index.html`). The Tauri custom protocol on macOS does not auto-append `index.html` the way dev servers do.

## Code Style

- **Warnings are errors.** Fix every compiler/linter warning before considering code done. Never leave warnings. Frontend: `pnpm run check` uses `--fail-on-warnings`. Rust: the root `Cargo.toml` sets `[workspace.lints.rust] warnings = "deny"` and every crate inherits it via `[lints] workspace = true` -- clippy and rustc warnings are build failures. New-stable clippy fallout is absorbed by documented allows in `[workspace.lints.clippy]`, not per-file attributes.
- **Reuse over reinvention.** Check `capsem-core` first. Extend existing abstractions.
- **Minimize code.** Delete dead code, inline single-use helpers. Every line must earn its place.
- **`capsem-core` is the shared library.** Service, process, CLI, and agent crates are thin shells. Business logic lives in core.
- **One way to do things.** Don't introduce a second pattern when one exists.
- **Rust tests live in a sibling `tests.rs`.** In the parent module declare `#[cfg(test)] mod tests;` and put all `#[test]` functions in `tests.rs` next to it. Never append an inline `mod tests { ... }` block at the bottom of a production file -- it buries prod code under scroll-past test fixtures and doubles the file size for every Read and grep. See `/dev-testing`.

## Invariants (do not break)

### Ephemeral VM model

**Everything is ephemeral unless asked otherwise.** VMs are temporary by default. Named VMs (`capsem create -n <name>`) are persistent -- workspace and rootfs overlay survive stops. `capsem create` is always detached; `capsem shell` is the interactive entry point (`capsem shell` with no args = temp VM + auto-destroy on exit).

**VirtioFS mode** (default): fresh workspace + sparse rootfs.img per session. Persistent VMs store their session in `~/.capsem/run/persistent/`.

**Block mode** (legacy): `mke2fs` unconditional at boot. Overlay upper is always tmpfs.

### Guest binary security

All guest binaries deployed chmod 555 (read-only). Rootfs mounted read-only. Guest cannot modify its own binaries.

### Codesigning

The binary must be codesigned with `com.apple.security.virtualization` or VZ calls crash. The justfile handles this.

## Bound Direct Diagnostics

Any direct development command that can block, build, launch children, or wait
on input must run through:

```text
python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds <finite> -- <command>
```

The wrapper closes stdin and owns a process group so timeout or interruption
cannot leave a Docker client, compiler, test runner, or helper behind. Do not
use it around `just test` or either release command: the gate's config-owned
timeouts, journal, resource teardown, and resumable graph remain authoritative.

## Serialized Orthogonal Releases

Release authority is [RELEASE.md](RELEASE.md). Start there, then load
`/release-process` for operational routing and hard-won implementation lessons.
The public entrypoints are:

```text
just release-binaries <channel> <source-commit>
just release-profile <channel> <profile> <source-commit>
```

Agents use these entrypoints rather than dispatching release workflows or
authoring manifests directly. Each release command is sufficient on its own:
its hosted lane performs release qualification, so `just test` is not a
prerequisite. `just test <source-commit>` is optional reusable complete local
verification. Direct release commands and `just test` own their timeouts,
journal, teardown, and network boundary; do not wrap or nest them.

Implementation-specific invariants belong beside their executable tests and in
the routed release references. If prose here disagrees with `RELEASE.md`, fix
the routing instead of creating another release model.

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

Checked-in first-party scripts have the same architectural backstop. New
scripts may not exceed `[boundary.scripts].max_lines`; larger historical files
are an exact line-count debt ratchet, not an exemption list. Split growth before
merging, and lower or remove a ratchet whenever a script shrinks. The guard
only inventories Git-tracked program sources under the configured first-party
roots, so generated outputs and vendored dependencies are outside its scope by
rule.

`just test` is **one process, one machine lock, one workspace, one plan**.
Its dry run reports the current totals; conditional asset staging makes a
checked-in count depend on machine state. It is diagnostic evidence, not a
prerequisite consumed by either release dispatcher.

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

## Vocabulary and gotchas

- **glowup** = installed-package release proof owned by `just test`: Linux runs `build_system/scripts/release/local-release-glowup.py` in Docker/systemd; macOS installs the signed exact package in Tart and boots it through physical Apple VZ.
- **winterfell** = service session-ledger lifecycle fixtures in `crates/capsem-service/src/tests.rs`; AGENTS.md's gate list refers to these.
- `just test` writes benchmark recordings under `target/test-benchmarks/`; intentional historical publication uses the owning benchmark command and explicit review.
- Rust is pinned to 1.97.1 in `rust-toolchain.toml`, bootstrap, CI, and Docker. Bump every surface together in a deliberate monthly toolchain PR and handle new-lint fallout there.

## Commits

1. Update `CHANGELOG.md` in the same commit **when the change is user-visible**.
   Refactors, test-only changes and internal cleanups do not need an entry.
   This used to read "every commit", which 39 of the last 100 did not do -- a
   rule nobody follows teaches that the neighbouring rules are advisory too,
   and the neighbours here are the DB boundary and the release contract.
2. Stage files explicitly (no `git add -A`)
3. Conventional subject, `type(scope): summary`. In use: `feat`, `fix`,
   `refactor`, `perf`, `test`, `docs`, `chore`, `style`, `security`, `merge`.
4. Author: Elie Bursztein <github@elie.net>
5. No `Co-Authored-By` trailers

## Logging

Boot sequence instrumented with `tracing` spans. `RUST_LOG=capsem=debug` for full timing, `RUST_LOG=capsem=info` for top-level.
