---
name: dev-gate
description: How capsem-gate works and how to add or change a gate command. Use when touching build, test, or release logic, or when a boundary/primitive/recursion/purity guard fails.
---

# The build and release gate

The justfile dispatches; `src/capsem/gate/` decides. No recipe carries a shell
body, none exceeds five lines, and both are contract tests rather than
conventions.

`just test` is **one process, one machine lock, one workspace, one plan**. Its
dry run reports the current totals; conditional asset staging makes a literal
step/action count depend on machine state. Both release commands *contain*
that same plan rather than launching it.

## The rule everything else follows from

**A plan action may never invoke `just` or another `capsem-gate` command.**

`GuardedRunner` refuses it at runtime, seeing through `uv run` and `caffeinate
env`. This is not style: the machine lock is not reentrant, so every such call
was a child waiting out its 7200-second timeout for the lock its own parent
held. Twenty-two of them existed, and each read perfectly at the call site —
`Run(["just", "_sign"])` looks like naming a step.

When you need another command's work, **compose its fragment**:

```python
def fragment(plan: Plan, config: GateConfig, *, after: tuple[Step, ...] = ()) -> Step:
    """Add this module's steps to `plan`; return its terminal step."""
```

Groundwork several fragments share uses `plan.shared(...)`, which makes the
second caller a dependant rather than a duplicate. **Pass `after` to the work,
not to the shared step** — sequencing shared groundwork behind one of its
consumers is a cycle, and one that only appears once two lanes compose.

A fragment that has more than one leaf returns *all* of them. `static` returned
only its last, so the phases after it started while a storage release it owned
was still outstanding.

## The interpreter comes first

`capsem-gate` is `capsem.gatelaunch:main`, not `capsem.gate.cli:main`. It
re-execs under a per-invocation `pycache_prefix` **before importing any of
`capsem.gate`**, and exports `PYTHONPYCACHEPREFIX` so pytest and every other
child inherits it. CPython validates a `.pyc` by mtime and size, so two
same-length edits inside one timestamp tick leave stale bytecode that still
looks current — which produced 74 false failures during one review, and which
would otherwise let a release qualify code that is not in the tree.

The complete gate refuses in `source.record` if `CAPSEM_GATE_PYCACHE` is
absent. If you drive a full plan from a test, set it.

Nothing in `gatelaunch` may import `capsem.gate` at module scope.

## Release state is one indivisible value

`Qualification.from_environment` is read once in `GateCommand.__init__` and
passed to `artifacts()`, `functional()` and `glowup()`. Three legal shapes —
local, binary release, profile release — and every partial combination is
refused while the plan is being built. **No module below reads
`CAPSEM_RELEASE_*` itself.** They each did, from a different variable, and a
dropped `GITHUB_ENV` line built a plan that verified pulled assets against a
source-rebuilt package.

In a test, hand the command an explicit `qualification=` rather than exporting
a variable.

## `execute()` enforces; you inherit it

Never overridden — a contract test fails if a subclass defines it. In order:

1. `plan()` is built with the machine **sealed** (`planseal.sealed()`). Ambient,
   not per-runner: `release.py` escaped an instance-scoped seal by constructing
   its own `Runner` inside `plan()`.
2. `plan.validate(config)` — cycles, declared exclusives, one owner per
   artifact. Before the lock, so a bad plan costs nothing.
3. `--graph` / `--dry-run` answered. **Before** `reexec()`, or asking becomes
   doing.
4. `reexec()`, outside the lock.
5. `RunLog.open` → `GuardedRunner` → `held(*resources)` → `plan.run(context)`.
6. `_summarize` — outside the log's context, so `run.end` is on disk first.

## The host kernel owns the network boundary

Candidate, both release commands, and directly invoked private test modules
run under Bubblewrap on Linux or Seatbelt on macOS. Linux receives loopback and
the configured UNIX sockets but no external interface; macOS uses the generated
profile. Linux `report` mode is deliberately refused because it cannot produce
the Seatbelt-style attempted-egress ledger.

`GateCommand` computes its effective typed sandbox mode once and exports it to
ordinary actions through the config-owned environment name. Outside-sandbox
actions clear it. Host Doctor combines owning-command policy with live kernel
state; the machine-lock marker proves only lock ownership, never enforcement.
Candidate and both release commands declare complete qualification through one
mixin. They accept only `enforce`: explicit `off` or `report` is refused before
plan construction, re-exec, or resource acquisition. Measure a changing rule
through a directly invoked incomplete module, whose evidence cannot be called
complete qualification.

A release cannot resolve or publish while trapped in that namespace, so it
starts one authenticated helper immediately before re-exec. Only `Run` and
`Script` actions declared with `outside_sandbox=True` receive that runner. The
mode-0600 metadata is consumed and deleted before the first plan action, and
the helper owns no plan, lock, workspace, or release state. Its commands still
pass through the same `GuardedRunner`, step log, journal, watcher checkpoints,
and re-entry refusal.

Exact guest base-image pulls are not helper egress. They are individually
recorded Docker CLI actions that materialize per-platform child manifests
through the already-declared Docker-daemon container-fetch boundary before the
sealed asset lanes consume them.

The allowed edges are exact and contract-tested: the shared fast module's
RustSec, npm bulk, and OSV actions; binary `channel-source`; both
`confirm-head` steps; and each final `release` step. The advisory authorities
are mutable security inputs and therefore cannot be replaced by locked package
prefetch. Never hand the external runner to any other qualification work.
Release CI downloads immutable inputs and materializes locked dependencies
before invoking its sandboxed modules.

## Declaring a command

```python
class MyCommand(GateCommand, name="my-command", help="one line for --help"):
    exclusive = True   # default is False; anything that WRITES needs True
    records = True     # False only for commands that read runs

    def resources(self):        # acquired in order, released in reverse
        return (Workspace(self._config),)

    def plan(self) -> Plan:
        plan = Plan(self.name)
        fragment(plan, self._config)
        return plan
```

Add the module to `COMMAND_MODULES` in `cli.py`.

**`exclusive` is about cross-process safety.** `[execution.exclusives]` entries
are `threading.Lock`s: they order steps inside one plan and coordinate nothing
between two `capsem-gate` processes. `just _sign` in one terminal could replace
the codesigned binaries a qualification in another was executing.

## Step or resource?

The question is *when it must happen*.

| | |
|---|---|
| **Step** | Work. Skipped when its dependency fails — which is right: it was written against something that was never produced. |
| **Resource** | Anything that must happen on **every** path including the aborted one. `held` releases in reverse; `preserve` runs on failure *before* release, because release destroys the evidence. |

The orphan-process count, the Colima lifecycle and the failure-evidence capture
are resources. The source-state check is a pair of steps — it must *not* run
when the gate failed, because the failure is the report.

## Everything is data

Every path, filename, architecture, channel **and environment variable name**
comes from `config/gate.toml`. `tests/test_gate_has_no_literal_data.py` catches
literals on both sides — a name read *and* a name used as a key in an
environment you hand to a process. Standard conventions (`HOME`, `TMPDIR`,
`PKG_CONFIG_PATH`, `HOST_UID`) are allowlisted; Capsem rails are not.

Reach for the builders rather than assembling dictionaries:
`config.environment.capsem(home=…, run_dir=…)`,
`config.environment.content(assets=…, profiles=…)`, and the typed families
under `[environment.package]`, `[environment.release_site]`,
`[environment.install_proof]`.

Treat a profile's assets and materialized configuration as one typed
`ProfileContent` root. Pass that value through composed package/install/glow-up
fragments; never rediscover its halves from ambient variables or mutable
checkout selectors at a Docker/Colima boundary.

Machine effects and stable lifecycle labels are closed `StrEnum` vocabularies.
Constructor seams accept those enum types, not `str`, and reject dynamic raw
strings at runtime. Keep a negative Ty fixture proving the wrong string does
not type-check; never widen the signature or raise the diagnostic ratchet to
make a new call fit. This is a correctness boundary: BuildKit and container
network modes once shared strings and sent `bridge` to `docker build`.

## Secrets are declared on the invocation

A `Command` that names credentials in `secret_env` cannot render them — not
through `str()`, not in the journal, not in the exception a failure raises. The
name survives, the value becomes `<redacted>`, and `evidence_argv` scrubs the
value out of argv too. Docker gets `-e NAME` and takes the value from its own
environment, because argv is readable through `ps` and no log filter covers
that.

## Every `Call` says why it is one

`Call` renders as prose, so a plan built from them says less. Pass
`why=Why.SECRETS` (one phase earns this: the package build), `Why.DYNAMIC`
(argv known only at run time) or `Why.COMPUTATION` — the last is a to-do,
not a design, and usually wants to become a named action.

## Stopping a run

`cancellation.check("what you are doing")` at points a partial unit can be
abandoned from — between files, between chunks, never mid-write. Ctrl-C sets
the switch, cancels pending steps, wakes lock waiters, and waits a bounded ten
seconds before naming whatever refused to stop. Pool workers do **not** inherit
the submitting context: `planrunner` hands the switch to `_guarded` explicitly.

## Asking without running

```bash
uv run capsem-gate <command> --dry-run    # every step, every action, real argv
```
```bash
uv run capsem-gate <command> --graph      # the same graph as mermaid
```
```bash
uv run capsem-gate runs last --failed     # what broke, where, how long
```
```bash
uv run capsem-gate gc --dry-run           # what disk the gate holds, per tree
```

`runs` and `gc` do not record themselves: `runs last` used to open a run and
repoint `latest` at the question.

## The guards that will fail you

| Test | What it holds |
|---|---|
| `test_gate_execute_funnel.py` | recursion refused; every subprocess logged; plan construction inert; isolation from acquired resources |
| `test_gate_no_nested_commands.py` | the same recursion rule statically, plus every named recipe and subcommand resolves |
| `test_gate_boundary.py` | no shell bodies; ≤5 recipe lines; ≤300 module lines; `ty` strict |
| `citadel/test_shape_boundaries.py` | every `[boundary.*]` source ceiling and its exact debt inventory |
| `test_gate_primitives_are_the_only_way.py` | only the harness touches the machine; only `planrunner` schedules |
| `test_gate_has_no_literal_data.py` | no path, architecture or channel spelled in code |
| `test_gate_hardening.py` | mutation is exclusive; plans are pure; verifications ask the real question |
| `test_gate_runlog_evidence.py` | attribution under concurrency; run status; non-recording inspection |
| `test_gate_lifecycle.py` | acquire order, reverse release, preserve first, primary error survives cleanup |
| `test_gate_qualification.py` | the three legal release states; every partial one refused |
| `test_gate_secrets.py` | no signing material in argv, journal, summaries or errors |
| `test_gate_source_identity.py` | the launcher; stale bytecode cannot be qualified |
| `test_gate_step_output.py` | each step keeps what its commands printed |
| `test_gate_cancellation.py` | Ctrl-C stops pending, running and waiting work |
| `test_just_argument_boundary.py` | every recipe parameter crosses one exact argv boundary |
| `test_gate_candidate.py` | the source state belongs to a run; observing a plan leaves the checkout alone |
| `test_gate_sandbox.py` | Bubblewrap/Seatbelt policy and the authenticated one-time release egress boundary |
| `test_gate_release_isolation.py` | only the named release edges may render outside the kernel sandbox |

## Testing a command

`tests/helpers/gate.py` is the one place that knows how to interrogate the
gate. Assert **edges**, not positions.

`gate_issued()` reads back real argv by *running* the plan against a recording
runner. That stubs subprocesses and nothing else, so it passes
`observing=True` and every primitive that touches the machine honours it. Any
new primitive that writes, deletes, links or hashes must check
`context.observing` first -- otherwise interrogating a plan mutates the
checkout a gate may be holding. `RecordSourceState` learned this the expensive
way: it overwrote the running gate's own state file with the recorder's empty
output, and `source.verify` -- the last step of a forty-minute run -- reported
a HEAD change on a tree nobody had touched.

The same rule applies to any contract that builds a plan from the real config
and runs it: pass `observing=True`. `tests/conftest.py` fails the test that
rewrites `target/gate-source-state.json`, so the next one to forget finds out
in seconds rather than at `source.verify`.

| | |
|---|---|
| `RecordingRunner` | records what a plan would run, for a test that drives its own plan |
| `gate_plan(name)` | a built plan; `after_of(label)` is how "these run in parallel" is asserted |
| `gate_labels(name)` | its step labels in graph order, for ordering claims |
| `gate_issues(name)` | real argv for everything a command would issue -- `None` reads the whole gate |

All three are cached. Contracts that used to grep the `justfile` for a command
use `gate_issues`; contracts about ordering use `gate_labels`. Do not grow a
local copy of these in a test file -- eight files did during the port, and the
copies drifted.

Two lessons paid for here:

**The double is not the thing.** `Resource.environment` is a method;
`Workspace.environment` was a property. Every funnel test passed because they
used a recorder written to match the protocol, and the one resource every
isolated command actually holds raised `TypeError`. Guards should walk the real
subclasses.

**A guard built from the current state asserts nothing.** The exclusivity guard
passed on first write because I listed what was already non-exclusive. Write the
claim, watch it fail, then make it true.

Break every guard once and watch it go red. Clear `__pycache__` between runs.

## The Citadel runs in the fast phase

`tests/citadel/` records architectural mistakes that must not be repeated. It
is source-level -- no artifact, no VM, no daemon -- so it is scheduled beside
Ruff and the source-syntax audit, not in the broad suite. It was reachable only
through the broad suite's `root` for a while, which meant a DB-boundary
violation surfaced after the VMs were up rather than in the first seconds.

`tests/citadel` is in `broad_ignores`: one collector, so its failures have one
owner. `citadel/test_guard_scheduling.py` fails if it is moved back behind the
expensive work.

Every guard states its reasoning in a named `*_RATIONALE` appended to the
assertion, so a violation teaches instead of printing a bare comparison. When
the reason is already stated somewhere canonical -- `[boundary]`, AGENTS.md, a
skill -- the rationale cites it rather than restating it; a second wording is a
second thing to keep in step.

| guard | what it holds |
|---|---|
| `test_db_boundary.py` | only `capsem-logger` executes ledger queries |
| `test_shape_boundaries.py` | every `[boundary.*]` ceiling and debt inventory |
| `test_workflow_enforcement.py` | a gating step cannot pass while failing |
| `test_container_workspace.py` | the guest builder's `/src/*` glob skips dotfiles |
| `test_skill_context_budget.py` | the skill description budget every session pays |
| `test_hot_build_contract.py` | hot codecs stay optimized in the dev profile |
| `test_package_architecture_boundary.py` | package and machine architecture never cross |
| `test_guard_scheduling.py` | the Citadel itself runs before the expensive work |

## See also

`/dev-just` for the public surface, `/dev-testing` for the suites,
`/release-process` for what the release lanes must guarantee.
