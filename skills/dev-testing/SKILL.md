---
name: dev-testing
description: Capsem testing policy and workflow. Use whenever running or writing tests. For VM, hypervisor, frontend, or Python specifics see the dev-testing-* skills.
---

# Testing

Read `tests/README.md` before adding or moving test fixtures. Test-only config
belongs under `tests/fixtures/`, not root `config/`.

## Where a new guard belongs

A test that records an architectural mistake -- something that must not be
repeated, checkable from source -- is a Citadel guard, in `tests/citadel/`, not
another file at `tests/` root. It runs in the fast phase, so it costs seconds
rather than waiting on an asset build.

Two rules the existing guards follow:

State the reason in a named `*_RATIONALE` appended to the assertion, so a
violation teaches. `test_db_boundary.py` is the model. Where the reason is
already stated canonically -- `config/gate.toml [boundary]`, AGENTS.md, a skill
-- cite it instead of restating it.

Write the adversarial case, not just the conformance case. A guard proved only
by inputs it was built for is not proved. `test_workflow_enforcement.py` carries
fourteen evasions and four legitimate shapes because enumerating bad spellings
let five through; the rule is now a whitelist for that reason.

## Test execution

| Command | What | VM? |
|---------|------|-----|
| `just fast-test` | Incomplete source feedback only | No |
| `just focus-test <group>` | One existing owner; `rust` selects changed crates and reverse dependents | Depends on group |
| `just install` | Complete local macOS package build and native install for hands-on testing | Yes |
| `just test` | Reusable complete local verification | Yes |

Release lanes are the publication authority and self-qualify; `just test` is optional.
`just fast-test` is incomplete feedback, and `just focus-test <group>` is the normal
targeted regression path. During TDD use the smallest native test; run `just test`
when complete local whole-system verification is useful.

The full gate is a construction boundary, not the edit loop. During TDD,
reproduce the failure with the smallest focused test, run that test red/green,
and batch adjacent parity fixes before paying for the complete gate. Run it
only when explicitly requested. Later fixes use focused owners; a gate-policy
edit is not permission for another full run. Release commands self-qualify and
do not require a developer-machine complete run.

`just test` deliberately accepts committed or uncommitted developer work. It
records `HEAD` plus a digest of every tracked and untracked non-ignored source
byte, then fails if either changes while the gate runs. Generated output stays
under ignored build directories. The local proof therefore covers the exact
source state the developer asked to test without forcing commit choreography.
Automatic gate benchmark output belongs under ignored
`cache/target/tests/benchmarks/`. Historical benchmark publication uses the owning
pytest/benchmark command and an explicit review; it is not a Just convenience
recipe.

## Release CI invariant

Release CI reuses the same checked-in private modules as local `just test`:

- `_test-fast`
- `_test-static`
- `_test-artifacts`
- `_test-functional`
- `_test-glowup`
- `_test-release-contracts`

The build scope is selective; the quality scope is not. Binary CI builds only
packages and resolves every channel profile by recorded digest. Profile CI
builds exactly one channel/profile and resolves the current package by recorded
digest. The resolved complementary artifacts are staged into the production
test harness, not replaced by source-built substitutes.

Every activated pairing must pass artifact validation, all VM suites,
Winterfell/MCP lifecycle, IronBank, injection, integration, benchmarks, full
`capsem-doctor`, exact native install, and update glow-up. A staged profile
whose minimum package is not yet satisfied may run only the self-consistency,
integrity, isolation, and boot proof; the following binary lane must run the
complete functional and glow-up modules before activation.

Both macOS and Linux must install their exact publishable native packages,
including their real post-install scripts, before publication. Notarization and
the public stable-to-nightly switch/upgrade glow-up remain mandatory
end-to-end proof.

On Apple Silicon macOS, `just test` owns the pre-publication macOS package
boundary through `build_system/packaging/macos/macos_release_glowup.py`: it builds the package with the
production assembler, installs that exact file in a disposable headless Tart
guest, verifies the receipt, app bundle, complete binary cohort, service and
gateway health, then extracts the same package on the physical Mac and boots a
real Capsem guest VM from its exact binary/profile payload to a shell marker.
The local package is unsigned; its postinstall ad-hoc signs the installed
Mach-O payload with the required entitlements. Local qualification must not
load Developer ID material or create a signing keychain. The tagged publication
workflow alone signs, notarizes, staples, and installs the final publishable
package.
Tart macOS guests do not support nested virtualization, so these are two
explicit halves of one script rather than a claimed nested proof. `just fast-test`
deliberately excludes Tart and therefore cannot be used for release.

Rust is pinned to `1.97.1` across the workspace file, workflow steps,
host-builder, and bootstrap. Change all pin surfaces together in a deliberate
toolchain-bump PR. RustSec and JavaScript bulk advisories are blocking in
`just fast-test`, local `just test`, ordinary CI, and both release lanes,
as well as the scheduled/manual audit. A new advisory fails the candidate
until it is remediated or explicitly reviewed in checked-in scanner policy.

Linux proof is host-aware: a cross-built non-host package receives structural
validation in qualification and exact native installation in its tagged release
job. `CAPSEM_REQUIRE_LINUX_DEB_PROOF=1` must not reject that non-host package
before the host package reaches its mandatory KVM proof. The hosted arm64 runner
does not expose `/dev/kvm`, so it proves exact package/service operation while
the x86_64 runner additionally owns the guest-shell marker.

Expensive harnesses need a cheap clean-environment bootstrap proof at the
start of `just test`, before Docker/Colima or artifact preparation. The one
private `_test-fast` module is also called by `just fast-test`, ordinary CI, and
both release lanes. It owns YAML/workflow and source syntax, source contracts,
dependency audits, Clippy, Python lint/type checks, and JavaScript/frontend
checks; no caller may reproduce a subset inline. Only a green fast gate may
build the Linux install-test image. That preflight must use a
container-owned `UV_PROJECT_ENVIRONMENT` and prove `python -m pytest` launches
before VM, package, or asset work consumes hours. Keep an ordering contract.
It is fail-fast infrastructure
validation only: the later Docker/systemd install E2E remains mandatory and
must still exercise the installed package and post-install behavior.

## Local/CI execution parity

Read `references/local-ci-parity.md` before editing any release workflow, gate
recipe, or CI job. It holds the Ironbank parity rule (every portable release
gate must be owned by `just test`), scanner/tool pinning, Docker platform and
prune discipline, common cache contracts, and the source-guard contracts.

## TDD workflow

Write tests first:
1. Write failing tests that capture expected behavior
2. Verify they fail for the right reason
3. Write minimal implementation to pass them
4. Refactor

Without a failing test first, it's easy to write tests that pass by accident or don't actually verify the behavior you intended.

## Functional slice proof matrix

Every non-trivial feature slice needs evidence in all of these categories before it can be called done. A green unit suite or a benchmark is not a substitute for functional or end-to-end proof.

| Category | What it proves | Minimum expectation |
|----------|----------------|---------------------|
| Unit/contract | Pure logic, parser state machines, schema migration, helper APIs | Red/green tests for normal and edge behavior at the smallest useful boundary |
| Functional | The feature works through its production-facing API, not just private helpers | Exercise the real module boundary with realistic inputs and assert outputs plus side effects |
| Adversarial | The feature preserves security, privacy, and policy invariants when attacked | Malformed, oversized, denied, missing, racing, timeout, permission, and leak-prevention cases |
| E2E/VM | The user-visible path works in a real Capsem session | Boot/run a VM or use the black-box CLI/MCP/service path, then inspect externally visible behavior |
| Telemetry | Audit data is present, accurate, and queryable | Query `session.db` or logger readers for required rows, fields, decisions, errors, and attribution |
| Performance | Hot paths stayed inside the accepted budget | Benchmarks or timing assertions with recorded numbers and regression criteria |

If a category is genuinely impossible or deliberately deferred, record it as missing with a reason, owner, and follow-up task. Silent deferral is the bug. "Covered by later E2E" is not enough unless the tracker names the later test and the current milestone is explicitly scoped as internal-only.

For policy, MITM, MCP, telemetry, networking, filesystem, process lifecycle, or sandbox-boundary work, the functional slice matrix is mandatory. The tests should prove not only that the happy path succeeds, but also that enforcement happens at the intended boundary: a blocked MCP tool does not dispatch, a blocked return does not leak, a denied URL does not reach the network, a malformed frame does not poison the stream, and telemetry records the truth.

## Ironbank ledger tests

Use `/ironbank` for release-critical VM, network, model, MCP, credential
broker, package-manager, doctor, benchmark, and security acceptance proof.
Ironbank lives in `tests/ironbank/` and is full black-box: tests are written
from public contracts, CLI help, docs, generated schemas, hermetic fixtures,
route responses, logs, DB rows, and installed package metadata. Do not inspect
Rust/product internals to decide expected behavior.

Ironbank cannot use:
- `skip`, `skipif`, `slow`, optional markers, or public-network dependencies
- status-code-only replay
- row-exists checks
- parser-only assertions
- manual OAuth/client runs as release proof

One deterministic stimulus must prove the whole ledger path: client result,
parsed facts, CEL/security decision, detection/enforcement rows, protocol DB
rows, structured logs, status counters, UDS route, HTTP route, and UI-facing
JSON. Every emitted DB/log/route field is exact-value asserted, covered by a
typed invariant, or explicitly marked not applicable. Unknown fields fail the
test until the field ledger is updated.

Package-manager tests prove function. Installing `zstd`, for example, means
compressing known bytes, decompressing them, and comparing the exact output;
not just checking dpkg output.

## Logged-data DB ownership

Telemetry and security ledgers are database-owned. Service routes, UI handlers,
MCP helpers, and benchmark harnesses must not build their own logged-data
projection caches and must not open SQLite directly. They may own query intent
(for example the fields a route needs), but they call the logger DB object to
execute it. The logger DB object owns connection threads, `mem`/disk table
layout, write buffering, flush, reload-from-disk behavior, WAL tuning, and
future FTS5/search tables.

Do not move route-specific SQL into `DbWriter` or turn the DB layer into a pile
of route helper methods such as `stats_detail_payload()` just to hide SQL. The
boundary is execution and storage mechanics:

```rust
db.ready().await?;
db.query(sql, params).await?;
db.write(event).await?;
```

`db.write(event).await` means the DB object accepted the event into its
producer buffer. Tests that assert read-after-write rows must use the DB flush
barrier or shutdown/reopen. Do not paper over visibility with sleeps, route
projections, or direct SQLite readers.

Empty table means empty result. Missing table or column means the schema
contract is broken and must fail loudly; never add compatibility branches that
treat missing ledger shape as empty data.

Regression tests must guard the boundary. If a route needs ledger data, add a
test that proves the route uses the DB object and a source guard that rejects
raw `rusqlite` opens, direct `DbReader::open`, and service-owned projection
state in production route code. Add a companion guard that prevents
route-specific DB writer methods or missing-schema fallbacks from being
introduced.

## Mock server boundary

`crates/capsem-mock-server` is the single reusable local fixture server for
benchmarks, doctor, protocol recording/replay, gateway/integration tests, and
Ironbank. It owns mock protocol responses and deterministic local upstream
behavior. Tests may launch it through `build_system/scripts/test/mock_server.py`,
`tests/helpers/mock_server.py`, or `CAPSEM_MOCK_SERVER_BASE_URL`.

Do not add another local HTTP/MCP/OAuth/model mock server for a feature. Extend
the shared mock server and its fixtures instead, then assert the route through
the relevant black-box test.

## Parallel tests as dogfooding (n=4 is non-negotiable)

`just test` runs the python suite under `pytest -n 4 --dist=loadfile`. Four real VMs boot simultaneously. **This is the canary, not just a speed-up.** We ship Capsem as a multi-VM sandbox for AI agents -- if our own test suite cannot safely boot 4 concurrent VMs, real users running an agent farm will hit the exact same bug. Treat any concurrency flake as a Capsem-side bug, not a test-tuning problem:

- "Suspend timed out" under load -> service IPC handling is racy, not "bump the timeout"
- "Session did not become ready" -> Apple VZ resource serialization, VirtioFS lock contention, or service handling concurrent provisions; investigate, don't suppress
- Two tests both want the same VM name -> name-collision bug in `validate_vm_name` / registry, not "isolate test names better"
- Stale socket between tests -> service didn't reap a child cleanly, real production bug

Anti-patterns when a test flakes under `-n 4`:
- Adding `time.sleep()` to "let things settle" -- masking a race
- Bumping the per-test timeout -- buying time for a real bug to manifest in prod instead of CI
- Marking the test `serial` so it runs alone -- defeating the dogfooding signal

The exception is a true timing or benchmark probe whose assertion is the
measured number. Those tests must already be marked `serial` and `just test`
runs them immediately after the `-n 4` canary. That is not a flake escape
hatch: it prevents another benchmark file from stealing the same Apple VZ
launch budget and corrupting the number we are trying to publish.

The host has plenty of headroom (48 GB RAM, 14 cores; 4 VMs at 2 GB / 2 CPU each = 8 GB / 8 cores). If concurrency surfaces a flake, fix the product, then re-run. Bumping `-n` higher (8, 12) is the natural follow-on once n=4 is stable -- real users will run more.

### Orphan processes across runs are a product bug (not a test bug)

If a previous `just test -n 4` run was interrupted (ctrl-C, pytest-xdist worker death, host crash) and the NEXT run flakes with "vm-ready never asserted", UDS "connection refused", or mysterious HTTP 500s -- the cause is companion processes from the interrupted run still alive under PID 1. `pkill -f "cache/target/cargo/debug/capsem-(service|process|gateway|tray|mcp)"` will make the flake vanish, but that is cleanup-after-the-fact. The fix is on the COMPANION side: every spawned companion (gateway, tray, and any new one) must use `capsem-guard::install(parent_pid, lock_path)` to enforce (a) refuse-standalone, (b) singleton, (c) self-exit on parent death. See `/dev-rust-patterns` lesson 18. Regression tests live in `tests/capsem-service/test_companion_lifecycle.py` -- never remove them; when adding a new companion, extend that file.

**Never `pkill -f capsem-` with a broad pattern** during test debugging: `capsem-` matches `--crate-name capsem-core` in running rustc/cargo invocations and will SIGKILL the compiler mid-build. Use a binary-path pattern like `pkill -f "cache/target/cargo/debug/capsem-(service|process|gateway|tray|mcp)"` instead.

### Apple VZ lifecycle serialization is part of the product

Apple's Virtualization.framework does not tolerate overlapping checkpoint
lifecycle operations (`saveMachineStateToURL` and `restoreMachineStateFromURL`)
on sibling VMs, and teardown must not cross those checkpoint edges. Capsem uses
`ServiceState::save_restore_lock` plus the host-wide `VzHostLock` flock:
cold starts and teardown take shared/read guards, save and restore take
exclusive/write guards. The rail holds even when pytest-xdist spawns one
`capsem-service` per worker, while independent cold starts can still run
together for the boot-latency gate.

Do not demote suspend/resume, lifecycle, provisioning, or teardown tests to
`-n 1` to sidestep VZ races. `just test` at `-n 4` is the contract; if a
concurrent run sees restore permission errors, loop-device corruption,
connection-refused startup races, or readiness misses, fix the lifecycle rail.
Full context and failure signatures live in
`web/docs/src/content/docs/gotchas/concurrent-suspend-resume.md`.

## Adversarial testing

Capsem is a security product. Every security-relevant feature needs tests that actively try to break invariants. Think like an attacker:
- Can a corp-blocked domain be snuck through another provider's list?
- Does an overlapping wildcard in allow+block always deny?
- Does malformed input (empty strings, unicode, huge payloads, invalid JSON) get rejected?
- Can path traversal escape the VirtioFS sandbox?
- Can a guest process modify its own binaries?

Stress-test boundary conditions. Write tests for the attacks you'd attempt yourself.

### Security invariants to verify in tests

When touching security-relevant code, check these invariants have test coverage:

| Invariant | What to test | Where |
|-----------|-------------|-------|
| VirtioFS share is `guest/` only | `session_dir/guest/` exists, symlinks resolve, host-only files (`session.db`, `serial.log`) are outside the share | `capsem-core::lib::tests` |
| UDS sockets are 0600 | After bind, verify permissions exclude other users | `capsem-process` |
| Process env is cleared | `env_clear()` called, only allowlisted vars passed | `capsem-service` spawn tests |
| No `process::exit` on guest I/O | Control channel close causes loop break, not exit | `capsem-process` |
| Sensitive logs are 0600 | `serial.log` created with restricted permissions | `capsem-process` |
| Gateway auth on all routes | Every route except `GET /` returns 401 without token | `capsem-gateway::auth::tests` |
| Auth rate limiting | 429 after threshold, resets after window | `capsem-gateway::auth::tests` |
| CORS rejects external origins | Only localhost/127.0.0.1/tauri allowed | `capsem-gateway::tests` |
| Body size limit | 413 for >10MB payloads | `capsem-gateway::proxy::tests` |
| VM ID validation | Path traversal (`../`), dots, spaces, null bytes rejected | `capsem-gateway::terminal::tests` |
| Rootfs read-only | profile rootfs asset mounted ro, guest binaries 555 | `capsem-doctor` in-VM tests |
| Suspend reports errors | IPC failure and timeout both return 500, not silent success | `capsem-service` tests |

## Test fixture anti-pattern: masking races with polling

If all test fixtures wait/poll before asserting, the tests will never catch server-side race conditions. For every endpoint that talks to a VM socket, write at least one test that calls it IMMEDIATELY after provision (no `wait_exec_ready`, no `ready_vm` fixture). The server must handle readiness internally.

**Pattern to avoid** (masks the bug -- server never needs wait logic because client always waits):
```
fixture calls provision -> fixture polls wait_exec_ready -> test calls exec
```

**Required test pattern** (catches the bug -- if server doesn't wait, test fails):
```
test calls provision -> test immediately calls exec -> server handles wait
```

See `tests/capsem-service/test_svc_exec_ready.py` for the regression tests that enforce this.

### wait_exec_ready is a single call, not a loop

`wait_exec_ready` (in `tests/helpers/service.py`, `tests/helpers/mcp.py`, `tests/capsem-gateway/test_gw_e2e.py`) makes one exec call with the server-side timeout passed through. The server's `handle_exec` calls `wait_for_vm_ready` internally, which polls until the VM is ready. Do NOT add client-side retry loops -- that creates a double-wait where each retry can block for the full server timeout (30s client retries x 30s server wait = pathological cascade). One wait, one place.

### Exec latency regression gate

`tests/capsem-serial/test_boot_timing.py::test_exec_latency_within_gate` asserts that provision-to-first-exec completes within `EXEC_LATENCY_GATE`. If this test fails, investigate boot time (process.log boot_timeline spans), not the wait mechanism.

## Where tests live

- **Rust unit: sibling `tests.rs` file, not inline `mod tests { ... }`.** See the next subsection.
- Rust integration: `crates/capsem-core/tests/`
- In-VM diagnostics: `guest/artifacts/diagnostics/test_*.py` (see dev-testing-vm)
- Hypervisor: KVM + Apple VZ tests (see dev-testing-hypervisor)
- Frontend: `web/app/src/lib/__tests__/` (see dev-testing-frontend)
- Python (builder): `tests/test_*.py`
- Python integration (service daemon): `tests/capsem-*/` directories, each with its own conftest.py and pytest marker
- Ironbank release ledger: `tests/ironbank/` (black-box only; no Rust
  implementation-derived expectations)

### Rust unit tests: sibling `tests.rs` pattern

**Every Rust module keeps its unit tests in a sibling `tests.rs`, not an inline `mod tests { ... }` block.** The parent module declares:

```rust
// foo.rs  OR  foo/mod.rs
// ... production code ...

#[cfg(test)]
mod tests;
```

and the tests go in `tests.rs` in the same directory:

```rust
// tests.rs -- sibling of foo.rs or child of foo/
use super::*;

#[test]
fn roundtrip() { ... }
```

**Why.** Inline `#[cfg(test)] mod tests { ... }` blocks are appended at the bottom of prod files and commonly hit 50–99% of the file's line count. That means every Read, grep, and scroll to reach production code walks past thousands of test lines first. Several modules in this codebase hit 4,000+ lines that way before extraction. Agents and humans both read faster when prod code isn't buried.

**Mechanics.**
- `tests.rs` is a submodule of the parent file -- `use super::*;` works, private items are visible, `#[cfg(test)]` on the `mod tests;` declaration still gates compilation.
- For files that don't yet have a sibling directory (e.g. `lib.rs`, `foo.rs`), put `tests.rs` next to them in the same `src/` directory.
- For files that are already `foo/mod.rs`, put `tests.rs` inside `foo/`.
- Attributes on the inline `mod tests` block (e.g. `#[allow(unused_imports)]`) move onto the declaration: `#[cfg(test)]\n#[allow(unused_imports)]\nmod tests;`.

**Extraction recipe** (for any remaining inline `mod tests { ... }`):
1. Move the block body (everything between the outer `{` and `}`) into a new sibling `tests.rs`.
2. Dedent one indentation level so contents read as top-level items.
3. Replace the old inline block with `#[cfg(test)] mod tests;` (plus any attributes that were on the original).
4. `cargo test -p <crate>` -- should pass identically.

**When to push back.** If you see a new PR or agent output adding an inline `mod tests { ... }` block, request it be moved to `tests.rs` before merge. Exceptions are narrow: tiny helper modules under ~50 lines total where inline tests plus prod code fit on one screen, or a module that's already a test-only helper.

### Source contracts must read the sibling, not the production file

A Python contract asserting that some Rust test *exists* must read the sibling
`tests.rs`, never the production `.rs` the test moved out of. Use
`tests/rust_sources.py`:

```python
from rust_sources import production, sibling_tests

assert "pub enum Status" in production(RELEASE_GRAPH)          # prod symbol
assert "release_graph_enums_reject_unknown" in sibling_tests(RELEASE_GRAPH)
```

Keep the two sources **separate**. Several contracts assert a symbol is
*absent* from production (`"Removed" not in source`), and a test module
legitimately names the thing it proves is rejected -- concatenating them lets a
fixture falsify a claim about shipped code.

`sibling_tests()` resolves `mod tests;` the way Rust does (`foo.rs` →
`foo/tests.rs`; `main.rs`/`lib.rs`/`mod.rs` → `tests.rs` beside it) and raises
when the module is missing rather than passing on an empty string. The helper is
not named `tests_of` on purpose: pytest collects `test*`, so the obvious name
becomes a phantom failing test in every importer.

`tests/test_rust_test_name_assertions.py` enforces this repo-wide and fails in
seconds. It resolves each assertion's target through the AST, per function
scope, so a contract that legitimately names a relocated test while asserting it
against a test module or spec document is not flagged.

**Why it matters.** This layout change broke sixteen contracts under
`tests/capsem-release/`, then five more under `tests/capsem_install/` that run
only inside the Docker install gate -- invisible until forty minutes into a
release run. Nothing about the failure pointed at a moved function; it read as a
broken release.

## Integration test suites

All Python integration tests live under `tests/capsem-*/` and use pytest markers. Each suite has a dedicated `just` recipe.

| Suite | Directory | Marker | VM? | What it tests |
|-------|-----------|--------|-----|---------------|
| Service API | `capsem-service/` | `integration` | Yes | HTTP endpoints: provision, list, info, exec, logs, file I/O, delete |
| CLI | `capsem-cli/` | `integration` | Yes | CLI subcommands via subprocess |
| MCP | `capsem-mcp/` | `mcp` | Yes | MCP server black-box (stdio, tool routing) |
| Session DB | `capsem-session/` | `session` | Yes | Telemetry: net/model/tool/mcp/fs/snapshot events |
| Snapshots | `capsem-snapshots/` | `snapshot` | Yes | Auto/manual snapshots, revert |
| Isolation | `capsem-isolation/` | `isolation` | Yes | Multi-VM filesystem + network isolation |
| Security | `capsem-security/` | `security` | Yes | Binary perms, codesigning, asset integrity, env blocklist |
| Config | `capsem-config/` | `config` | Yes | Limits, resource bounds, hot-reload |
| Bootstrap | `capsem-bootstrap/` | `bootstrap` | No | Setup flow, dev tools, asset checks |
| Stress | `capsem-stress/` | `stress` | Yes | 5 concurrent VMs, rapid create/delete |
| Build chain | `capsem-build-chain/` | `build_chain` | Yes | cargo build -> codesign -> pack -> manifest -> boot |
| Guest | `capsem-guest/` | `guest` | Yes | Network, services, filesystem, env inside guest |
| Cleanup | `capsem-cleanup/` | `cleanup` | Yes | Process killed, socket removed, session dir removed |
| Codesign | `capsem-codesign/` | `codesign` | No | All binaries signed, entitlements present (FAIL not skip) |
| Serial | `capsem-serial/` | `serial` | Yes | Console logs, boot timing < 30s |
| Session lifecycle | `capsem-session-lifecycle/` | `session_lifecycle` | Yes | DB exists, schema, events, survives shutdown |
| Config runtime | `capsem-config-runtime/` | `config_runtime` | Yes | CPU/RAM applied in guest, blocked domains |
| Recipes | `capsem-recipes/` | `recipe` | No | just run-service, just doctor, cargo build |
| Recovery | `capsem-recovery/` | `recovery` | Yes | Stale socket/instances, orphaned process, double service |
| Rootfs artifacts | `capsem-rootfs-artifacts/` | `rootfs` | No | Artifact files, build context, doctor consistency |
| Session exhaustive | `capsem-session-exhaustive/` | `session_exhaustive` | Yes | Per-table data validation, cross-table FK integrity |
| Install | `capsem_install/` | `install` | No | Native package installer: layout, auto-launch, service install, manifest placement, update, uninstall, lifecycle, reinstall, error paths |

`just test` is the only public complete local verification. `just fast-test`
is incomplete feedback, `just focus-test` selects a closed existing owner, and
`just install` is the complete local product install. Do not add another
composite; use the owning native test for narrower diagnosis.

Public Just recipes, Capsem CLI command paths, and service HTTP method/path
pairs are exact approval-gated surfaces. Any change must pass
`tests/test_public_surface_contract.py` and requires explicit approval before
editing `config/public-surface.toml`.

## Test matrix and coverage

Read `references/test-matrix.md` for the per-crate Rust CI matrix, coverage
enforcement, and Python suite tiers. Rust workspace and per-crate floors are
owned only by `config/gate.toml`; the per-crate checker rejects stale headroom,
so meaningful coverage gains raise their ratchets in the same change.

## Fast debug with capsem MCP tools

Read `references/mcp-debug-tools.md` for interactive VM debugging through the
capsem MCP server: tool table, one-shot vs iterative workflows, and common
session-DB queries. MCP tools are for fast targeted checks; just recipes are
for comprehensive validation before committing.

## End-to-end validation is not optional

After any change touching guest binaries, network policy, telemetry, MCP, or VM lifecycle:

1. `just exec "capsem-doctor"` -- verifies sandbox integrity inside the VM
2. After telemetry/logging changes: run a real session and verify with `python3 build_system/scripts/doctor/check_session.py` that net_events, model_calls, tool_calls, tool_responses, fs_events, dns_events, and security_rule_events are populated correctly for the exercised protocols

## When tests fail

Never dismiss a test failure as "pre-existing" or "unrelated." Every failure must be investigated. Follow the dev-debugging workflow:

1. **Do not change the test to make it pass.** The test is evidence. Changing the assertion to match broken behavior destroys that evidence.
2. **Reproduce and diagnose first.** Understand *why* it fails before writing any fix. See the dev-debugging skill for the full methodology: reproduce with a test, diagnose root cause, then fix comprehensively.
3. **Fix the code, not the test.** If the test is genuinely wrong (not the code), explain in detail why the test's expectation is incorrect before changing it.

### Measuring a gate's result

**Never take the last line of a multi-part result as the result.** Two shapes of
one mistake, both of which report success while the thing measured failed:

**`$?` after a pipe is the pipe's status.** `just test | tail` reports what
`tail` did. Redirect, then read the code separately:

```bash
just test > /tmp/gate.log 2>&1; echo "EXIT=$?"
```

**`tail -n1` across a multi-part result returns the last part, not the whole.**
`cargo test -p capsem-service` runs three test binaries; the last prints
`0 passed`, so `| tail -1` reads as though the crate had no tests while 91 and
264 passed above it. Aggregate instead of sampling:

```bash
cargo test -p capsem-service 2>&1 | grep -E "^test result:"   # every binary
```

Both errors are silent and both flatter you: one turns a failed gate into a
pass, the other turns a passing crate into a phantom regression. If a command
can emit more than one verdict, read them all.

Read the *first* real error, not the recipe cascade under it — `grep -aE "^FAILED|^E "` lands on the cause, while the trailing `error: Recipe ... failed` lines are only the unwind.

`build_system/tests/gate/test_exit_status_integrity.py` keeps this out of committed recipes,
scripts, and workflows, and requires `set -o pipefail` in any bash recipe that
pipes. It cannot see an agent's ad-hoc shell — that part is on you.

### Fixtures use the wrapper, never the raw variable

Redirect Capsem paths with `paths::CapsemPathsGuard::redirect(root)`. It sets
`CAPSEM_HOME`, `CAPSEM_RUN_DIR`, and `CAPSEM_ASSETS_DIR` from one root and
restores on drop, so a fixture cannot set one and inherit the rest.

Read logs with `telemetry::read_log_tail`, including in assertions: a test that
opens a `*.log` path directly stops exercising what the product does the moment
that stream rotates.

Both are enforced by `build_system/tests/gate/test_path_and_log_wrappers_are_mandatory.py`. See
`/dev-rust-patterns` "One rule, one function" for why.

### Verify with the gate's environment, not a bare shell

`just test` exports `CAPSEM_HOME`, `CAPSEM_RUN_DIR`, `CAPSEM_TEST_PROFILE`, and
`CAPSEM_BENCHMARK_OUTPUT_ROOT`. A test that reads ambient state passes in your
shell and fails in the gate:

```bash
CAPSEM_HOME="$PWD/cache/target/tests/home/.capsem" \
CAPSEM_RUN_DIR="$PWD/cache/target/tests/home/.capsem/run" \
  cargo test -p <crate>
```

A fixture that overrides `CAPSEM_HOME` must override `CAPSEM_RUN_DIR` too —
the run dir takes precedence over the home-derived default, so setting only the
first sends production code to the ambient run directory while the fixture
writes into a temp one. Bisect by exporting one variable at a time; that names
the culprit in two runs instead of guessing.

### Cache limits belong in the cache contract

A number copied next to a rule drifts from it silently. Three separate gate
failures in one session traced to this: a coverage floor copied into a test, a
guest kernel major copied beside its pin, and a Docker fixture restating a
storage assumption outside the cache policy.

Each read as a broken product, and each surfaced minutes-to-an-hour into a gate rather than at the edit. Derive the value from its source, or name it once and pin config and contract together:

```python
maximum = load_policy(ROOT).runtimes["docker"].max_size_bytes
assert registry.contract("docker").max_size_bytes == maximum
```

Prove it derives rather than hardcodes: change the source value and confirm the test *follows* instead of breaking.
Read `/dev-cache` for the common cache model; tests must not recreate backend
accounting or machine-capacity rails.

## Platform gating tests

`cargo test --test platform_gating` scans all `.rs` files under `crates/` for macOS-only and Linux-only symbols (`libc::clonefile`, `AppleVzHypervisor`, `KvmHypervisor`, `FICLONE`, etc.) and verifies they appear inside `#[cfg(target_os = "...")]` blocks. This catches ungated platform APIs before they reach CI. Run this test when adding any platform-specific code.

## Excluding something from a guard

Real trees contain things a guard should not fail on. One legal shape, in
`capsem.gate.exclusions`: **exact** (the thing, not a category), **hashed**
where the subject is content, **with a stated reason** whose length the schema
checks, and **reconciled both ways** so a stale entry fails too. Never a count
-- it fails on a harmless addition and passes on a dangerous change to
something already listed. `/dev-gate` has why each wrong shape was tried.

## Testable design

Extract logic from presentation and process entrypoints into the
lowest-dependency crate that owns its domain. If logic cannot be tested without
booting a VM or launching the GUI, first separate the pure decision from its
runtime adapter; do not default unrelated shared code into `capsem-core`.
